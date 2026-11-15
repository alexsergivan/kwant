// Streaming harness runner (dev only; perf/stream/README.md). Serves perf/stream/dist, opens it in headless
// Chromium, streams tokens into a chat of --n messages and prints what the stream cost:
//   commits/s    React commits per second during the stream (a devtools hook counts them)
//   scripting    CDP ScriptDuration over the stream, in ms
//   longest frame  the longest gap between animation frames during the stream, in ms (plus how many went over 50 ms)
// Usage: node perf/stream/run.mjs --n 300 --tokens 2000 --rate 50 [--width 1280] [--shot file.png] [--label after]
// Needs playwright-core and a Chromium: PLAYWRIGHT_CORE=/path/to/playwright-core/index.mjs CHROMIUM=/path/to/chrome
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const N = Number(arg('n', 300));
const TOKENS = Number(arg('tokens', 2000));
const RATE = Number(arg('rate', 50));
const WIDTH = Number(arg('width', 1280));
const SHOT = arg('shot', '');
const LABEL = arg('label', '');
// 15a: --end-shot after the last token (still streaming), --final-shot once the saved reply has landed.
const END_SHOT = arg('end-shot', '');
const FINAL_SHOT = arg('final-shot', '');
// 16a: --measure file.json writes the reply's geometry at the last token and once landed (bubble, blocks).
const MEASURE = arg('measure', '');
const { chromium } = await import(process.env.PLAYWRIGHT_CORE ?? '/home/maus/work/iris-ft-shots/node_modules/playwright-core/index.mjs');
const exe = process.env.CHROMIUM ?? '/home/maus/.cache/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-linux-arm64/chrome-headless-shell';

const dist = path.join(path.dirname(fileURLToPath(import.meta.url)), 'dist');
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
const ctx = await browser.newContext({ viewport: { width: WIDTH, height: phone ? 844 : 800 }, deviceScaleFactor: phone ? 2 : 1, isMobile: phone, hasTouch: phone, colorScheme: 'dark', reducedMotion: 'reduce' });
const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.addInitScript(() => {
  // React DOM reports each commit to a devtools hook when one is there (production builds too).
  const m = { commits: 0, on: false, frames: [], last: 0 };
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
await page.goto(`${base}/?n=${N}`);
await page.waitForFunction(() => window.__harness?.ready(), null, { timeout: 120000 });
await page.waitForTimeout(1500);
const rows = await page.evaluate(() => document.querySelectorAll('[data-message-id]').length);
const cdp = await ctx.newCDPSession(page);
await cdp.send('Performance.enable');
const metric = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
const m0 = await metric();
await page.evaluate(() => Object.assign(window.__m, { on: true, commits: 0, frames: [], last: 0 }));
const shotAt = SHOT ? Math.floor(TOKENS * 0.6) : 0;
let sent;
if (shotAt) {
  await page.evaluate(([c, r]) => window.__harness.stream(c, r), [shotAt, RATE]);
  await page.screenshot({ path: SHOT });
  sent = await page.evaluate(([c, r]) => window.__harness.stream(c, r), [TOKENS - shotAt, RATE]);
  sent.sent += shotAt;
} else if (TOKENS === 0) {
  // --tokens 0: the same 40 s with nothing streaming, to see what the machine itself costs.
  await page.waitForTimeout(40000);
  sent = { sent: 0, ms: 40000 };
} else sent = await page.evaluate(([c, r]) => window.__harness.stream(c, r), [TOKENS, RATE]);
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const m1 = await metric();
const out = await page.evaluate(() => {
  const m = window.__m;
  m.on = false;
  const over50First = m.frames.slice(0, Math.floor(m.frames.length / 2)).filter((x) => x > 50).length;
  // The reply's second half (by frame order): where a growing reply costs most (15a).
  const second = m.frames.slice(Math.floor(m.frames.length / 2));
  const f = m.frames.slice().sort((a, b) => a - b);
  const pct = (p) => f[Math.min(f.length - 1, Math.floor((f.length * p) / 100))] ?? 0;
  // The streamed text's last words are on screen once the stream ends (nothing late or lost).
  const words = window.__harness.streamed().match(/[A-Za-z]{5,}/g) ?? [];
  const tail = words.slice(-3).join(' ');
  const shown = [...document.querySelectorAll('.markdown-content')].map((e) => e.textContent).join(' ').match(/[A-Za-z]{5,}/g) ?? [];
  return { over50First, commits: m.commits, frames: f.length, longest: f[f.length - 1] ?? 0, p95: pct(95), over50: f.filter((x) => x > 50).length, tail, tailShown: shown.join(' ').includes(tail), secondFrames: second.length, secondLongest: Math.max(0, ...second) };
});
const secs = sent.ms / 1000;
const result = {
  label: LABEL,
  messages: N,
  rows,
  width: WIDTH,
  tokens: sent.sent,
  streamSeconds: +secs.toFixed(2),
  commitsPerSecond: +(out.commits / secs).toFixed(1),
  commits: out.commits,
  scriptingMs: Math.round((m1.ScriptDuration - m0.ScriptDuration) * 1000),
  taskMs: Math.round((m1.TaskDuration - m0.TaskDuration) * 1000),
  layoutMs: Math.round((m1.LayoutDuration - m0.LayoutDuration) * 1000),
  styleMs: Math.round((m1.RecalcStyleDuration - m0.RecalcStyleDuration) * 1000),
  longestFrameMs: Math.round(out.longest),
  p95FrameMs: Math.round(out.p95),
  framesOver50: out.over50,
  over50FirstHalf: out.over50First,
  over50SecondHalf: out.over50 - out.over50First,
  secondHalfFrames: out.secondFrames,
  secondHalfLongestMs: Math.round(out.secondLongest),
  over50SecondHalfPct: out.secondFrames ? +((100 * (out.over50 - out.over50First)) / out.secondFrames).toFixed(2) : 0,
  load: os.loadavg().map((x) => +x.toFixed(2)),
  frames: out.frames,
  lastWordsShown: out.tailShown,
  errors,
};
console.log(JSON.stringify(result));
if (END_SHOT) await page.screenshot({ path: END_SHOT });
// The bubble's box and each text block's box relative to it: same x and heights live and landed = no shift, no re-wrap.
const geometry = (sel) =>
  page.evaluate((sel) => {
    const md = document.querySelector(sel);
    const bubble = md?.closest('[data-live-bubble-body]') ?? md?.parentElement;
    if (!bubble) return null;
    const b = bubble.getBoundingClientRect();
    const r = (n) => Math.round(n * 100) / 100;
    const blocks = [...bubble.querySelectorAll('p, li, h1, h2, h3, h4, pre, table, blockquote')].map((el) => {
      const e = el.getBoundingClientRect();
      return { tag: el.tagName.toLowerCase(), x: r(e.left), w: r(e.width), h: r(e.height), dy: r(e.top - b.top), lines: el.getClientRects().length };
    });
    return { bubble: { x: r(b.left), w: r(b.width), h: r(b.height) }, text: blocks[0] ? { x: blocks[0].x } : null, blocks };
  }, sel);
const live = MEASURE ? await geometry('[data-live-bubble-body] .markdown-content') : null;
if (FINAL_SHOT || MEASURE) {
  await page.evaluate(() => window.__harness.finish());
  await page.waitForSelector('[data-message-id="m-final"]', { timeout: 30000 });
  await page.waitForFunction(() => !document.querySelector('[data-live-bubble-body] .markdown-content'), null, { timeout: 30000 }).catch(() => {});
  await page.waitForTimeout(800);
  if (FINAL_SHOT) await page.screenshot({ path: FINAL_SHOT });
}
if (MEASURE) {
  const landed = await geometry('[data-message-id="m-final"] .markdown-content');
  const same = !!live && !!landed && live.blocks.length === landed.blocks.length;
  const moved = same ? live.blocks.filter((l, i) => ['x', 'w', 'h', 'dy'].some((k) => l[k] !== landed.blocks[i][k])).length : null;
  fs.writeFileSync(MEASURE, JSON.stringify({ width: WIDTH, label: LABEL, live, landed, blocksMoved: moved }, null, 1));
}
await browser.close();
server.close();
