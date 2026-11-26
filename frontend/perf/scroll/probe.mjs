// chat-scroll-holds-place probe (dev only; perf/scroll/README.md). Opens the stream harness's long mixed chat
// (perf/stream, ?mixed=1&paged=1: 300 messages, ten smart results, long replies with a table and code, a question
// card, 100-message pages) and checks the rule: while the person reads (they scrolled away from the bottom), nothing
// moves the messages on screen but their own input.
//
// Every frame it logs the top of the message nearest the middle of the chat, measured from the scroller's top. A
// jump is a change of more than 4 px in a frame with no input from the probe in it or the frame before (wheel,
// touch, a held pointer, a scrolling key). The anchor message is picked again after each input.
//
// Usage (from frontend/, after `npx vite build --config perf/stream/vite.config.mts`):
//   node perf/scroll/probe.mjs [--engine chromium|webkit|webkit-old] [--width 1280|390] [--scenario 1..7|all]
//                              [--stream on|off|both] [--dist perf/stream/dist] [--strip out.png] [--json out.json]
// webkit-old is WebKit with scroll anchoring and 33q's content-visibility forced off, like an older Mac WebView
// (the 33q rule only turns content-visibility on where `overflow-anchor` is supported).
// Needs playwright-core and its browsers: PLAYWRIGHT_CORE=/path/to/playwright-core/index.mjs, CHROMIUM=/path/to/chrome,
// WEBKIT=/path/to/pw_run.sh (each optional). Exit code 1 when any run has a jump.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const here = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(arg('dist', path.join(here, '..', 'stream', 'dist')));
const ENGINES = arg('engine', 'chromium,webkit,webkit-old').split(',');
const NOLAZY = process.argv.includes('--nolazy');
const WIDTHS = arg('width', '1280,390').split(',').map(Number);
const SCEN = arg('scenario', 'all');
const STREAMS = { on: [true], off: [false], both: [false, true] }[arg('stream', 'both')];
const STRIP = arg('strip', '');
const JSON_OUT = arg('json', '');
const N = Number(arg('n', 300));
const pw = await import(process.env.PLAYWRIGHT_CORE ?? path.resolve(here, '../../node_modules/playwright-core/index.mjs'));

const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png' };
const server = http.createServer((req, res) => {
  const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = path.join(DIST, p === '/' ? 'index.html' : p);
  if (!file.startsWith(DIST) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return res.writeHead(404).end();
  res.writeHead(200, { 'Content-Type': types[path.extname(file)] ?? 'application/octet-stream' }).end(fs.readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

// ---- in the page: the frame logger ----
function monitor() {
  // `quiet`: frames still counted as the input's own (2 after it, longer while the scroll it started still moves,
  // at most 12). Nothing counts before the first input: until then the chat follows its bottom, as it should.
  const P = { on: false, reading: false, quiet: 0, budget: 0, lastTop: null, anchor: null, last: null, frames: 0, jumps: [], log: [] };
  window.__probe = P;
  const editable = (t) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
  // dir: 1 when the input scrolls up (the messages move down), -1 down, 0 unknown. In the input's own frames only a
  // move against its direction counts (the chat pulling the reader back while they scroll).
  const input = (dir = 0) => {
    P.quiet = 2;
    P.budget = 12;
    P.dir = dir;
    if (P.on) P.reading = true;
    // The anchor's place as the input lands: a pin made just before it (the chat following a streamed word, as it
    // should while the reader was at the bottom) is not the chat moving against the input.
    const sc = P.scroller?.();
    if (P.on && sc && P.anchor?.isConnected) P.atInput = P.anchor.getBoundingClientRect().top - sc.getBoundingClientRect().top;
  };
  let touchY = null;
  addEventListener('wheel', (e) => input(e.deltaY < 0 ? 1 : -1), { capture: true, passive: true });
  addEventListener('touchstart', (e) => ((touchY = e.touches[0]?.clientY ?? null), input(0)), { capture: true, passive: true });
  addEventListener('touchmove', (e) => {
    const y = e.touches[0]?.clientY ?? null;
    input(touchY !== null && y !== null && y !== touchY ? (y > touchY ? 1 : -1) : 0);
    touchY = y;
  }, { capture: true, passive: true });
  for (const t of ['pointerdown', 'mousedown']) addEventListener(t, () => input(0), { capture: true, passive: true });
  addEventListener('pointermove', (e) => e.buttons && input(0), { capture: true, passive: true });
  const KEY = { PageUp: 1, ArrowUp: 1, Home: 1, PageDown: -1, ArrowDown: -1, End: -1, ' ': -1 };
  addEventListener('keydown', (e) => !editable(e.target) && e.key in KEY && input(e.shiftKey && e.key === ' ' ? 1 : KEY[e.key]), { capture: true, passive: true });
  P.input = input;
  const scroller = () => document.querySelector('[role="log"][aria-label^="Messages in"]');
  P.scroller = scroller;
  const nearestMiddle = (sc) => {
    const box = sc.getBoundingClientRect();
    const mid = (box.top + box.bottom) / 2;
    let best = null;
    let bestD = Infinity;
    for (const r of sc.querySelectorAll('[data-message-id]')) {
      const b = r.getBoundingClientRect();
      if (b.bottom < box.top || b.top > box.bottom) continue;
      const d = Math.abs((b.top + b.bottom) / 2 - mid);
      if (d < bestD) (bestD = d), (best = r);
    }
    return best;
  };
  // Read once each frame has been drawn: a task posted from requestAnimationFrame runs after that frame's layout,
  // ResizeObservers and paint. Read in requestAnimationFrame itself (before them), a row that settled and the
  // correction the chat makes for it in the same frame would count as a jump nobody saw.
  const measure = (t) => {
    const sc = scroller();
    if (P.on && sc) {
      P.frames++;
      const prevTop = P.lastTop;
      const moved = P.lastTop !== null && Math.abs(sc.scrollTop - P.lastTop) > 0.5;
      P.lastTop = sc.scrollTop;
      if (P.quiet === 0 && P.budget > 0 && moved) P.quiet = 1;
      const inputFrame = !P.reading || P.quiet > 0;
      if (P.quiet > 0) P.quiet--;
      if (P.budget > 0) P.budget--;
      const box = sc.getBoundingClientRect();
      let row = P.anchor && P.anchor.isConnected ? P.anchor : null;
      // Against the input: from the last frame and from the moment of input (so not a pin that landed just before it).
      const back = (from) => from === null || from === undefined || (row.getBoundingClientRect().top - box.top - from) * P.dir < -4;
      const against = inputFrame && P.reading && P.dir !== 0 && row && P.last !== null && back(P.last) && back(P.atInput);
      P.atInput = null;
      if (!row || (inputFrame && !against)) {
        row = nearestMiddle(sc);
        P.anchor = row;
        P.last = null;
      }
      if (row) {
        const top = row.getBoundingClientRect().top - box.top;
        if (P.last !== null && (!inputFrame || against) && Math.abs(top - P.last) > 4)
          P.jumps.push({ t: Math.round(t), dy: Math.round(top - P.last), id: row.getAttribute('data-message-id'), scrollTop: Math.round(sc.scrollTop), scrolled: Math.round(sc.scrollTop - (prevTop ?? sc.scrollTop)), gap: Math.round(sc.scrollHeight - sc.scrollTop - sc.clientHeight), what: P.what, against: !!against });
        if (against) {
          row = nearestMiddle(sc);
          P.anchor = row;
          P.last = row ? row.getBoundingClientRect().top - box.top : null;
          return;
        }
        P.last = top;
      }
    }
  };
  const ch = new MessageChannel();
  ch.port1.onmessage = (e) => measure(e.data);
  const loop = (t) => {
    ch.port2.postMessage(t);
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function open(engine, width, n = N) {
  const phone = width < 500;
  const browser =
    engine === 'chromium'
      ? await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox', '--disable-smooth-scrolling'], ignoreDefaultArgs: ['--hide-scrollbars'] })
      : await pw.webkit.launch({ executablePath: process.env.WEBKIT || undefined });
  const ctx = await browser.newContext({ viewport: { width, height: phone ? 844 : 800 }, deviceScaleFactor: 1, isMobile: phone && engine === 'chromium', hasTouch: phone, colorScheme: 'dark', reducedMotion: 'no-preference' });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => !/audio device/.test(String(e)) && errors.push(`${e}`.slice(0, 120) + (e.stack ? ` @ ${e.stack.split('\n')[1]?.trim()}` : '')));
  await page.addInitScript(monitor);
  // webkit-old: no scroll anchoring and no 33q (as in an older Mac WebView); --nolazy: 33q off only, to measure it.
  const css = [engine === 'webkit-old' && '[role="log"]{overflow-anchor:none!important}', (engine === 'webkit-old' || NOLAZY) && '.kw-row-lazy{content-visibility:visible!important;contain-intrinsic-size:none!important}'].filter(Boolean).join(' ');
  if (css)
    await page.addInitScript((css) => {
      const s = document.createElement('style');
      s.textContent = css;
      document.addEventListener('DOMContentLoaded', () => document.head.appendChild(s));
    }, css);
  await page.goto(`${base}/?n=${n}&mixed=1&paged=1`);
  await page.waitForFunction(() => window.__harness?.ready() && window.__probe.scroller(), null, { timeout: 120000 });
  await sleep(1500);
  return { browser, page, phone, errors };
}

// ---- input helpers ----
const box = (page) => page.evaluate(() => { const r = window.__probe.scroller().getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
async function wheel(page, dy) {
  const b = await box(page);
  await page.mouse.move(b.x + b.w / 2, b.y + b.h / 2);
  await page.mouse.wheel(0, dy);
}
async function touchScroll(page, engine, dy) {
  // dy < 0 scrolls up (the finger moves down).
  const b = await box(page);
  if (engine === 'chromium') {
    // Raw touch points, 20 px a frame: the headless shell's synthesizeScrollGesture sends touchstart/touchend only
    // and never scrolls, so a probe built on it saw the reader "pulled back" to a bottom they never left.
    const cdp = await page.context().newCDPSession(page);
    const x = Math.round(b.x + b.w / 2);
    const steps = Math.ceil(Math.abs(dy) / 20);
    let y = Math.round(b.y + b.h / 2 + (dy < 0 ? -1 : 1) * Math.min(b.h / 2 - 20, steps * 10));
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y }] });
    for (let i = 0; i < steps; i++) {
      y -= Math.sign(dy) * 20;
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x, y }] });
      await sleep(16);
    }
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await cdp.detach();
    return;
  }
  // WebKit has no synthesized touch scrolling here (nor a Touch constructor on Linux): page-made touch events
  // with their touches set by hand, and the scroll they would make, 20 px a frame.
  await page.evaluate(async (dy) => {
    const sc = window.__probe.scroller();
    const r = sc.getBoundingClientRect();
    const fire = (type, y) => {
      const e = new Event(type, { bubbles: true, cancelable: false });
      const touches = y === null ? [] : [{ identifier: 1, target: sc, clientX: r.x + r.width / 2, clientY: y }];
      Object.defineProperty(e, 'touches', { value: touches });
      Object.defineProperty(e, 'changedTouches', { value: touches });
      sc.dispatchEvent(e);
    };
    let y = r.y + r.height / 2;
    fire('touchstart', y);
    const steps = Math.ceil(Math.abs(dy) / 20);
    for (let i = 0; i < steps; i++) {
      y -= Math.sign(dy) * 20;
      fire('touchmove', y);
      sc.scrollTop += Math.sign(dy) * 20;
      await new Promise((f) => requestAnimationFrame(f));
    }
    fire('touchend', null);
  }, dy);
}
async function dragThumb(page, dyPx, steps = 10) {
  const g = await page.evaluate(() => {
    const sc = window.__probe.scroller();
    const r = sc.getBoundingClientRect();
    const thumbH = Math.max(20, (sc.clientHeight * sc.clientHeight) / sc.scrollHeight);
    const top = r.top + (sc.scrollTop / (sc.scrollHeight - sc.clientHeight)) * (sc.clientHeight - thumbH);
    return { x: r.right - 3, y: top + thumbH / 2, before: sc.scrollTop };
  });
  await page.mouse.move(g.x, g.y);
  await page.evaluate(() => window.__probe.input());
  await page.mouse.down();
  // A scrollbar drag sends no events to the page in every engine: the probe marks its own input.
  for (let i = 1; i <= steps; i++) {
    await page.evaluate((d) => window.__probe.input(d), dyPx < 0 ? 1 : -1);
    await page.mouse.move(g.x, g.y + (dyPx * i) / steps);
    await sleep(40);
  }
  await page.evaluate(() => window.__probe.input());
  await page.mouse.up();
  const after = await page.evaluate(() => window.__probe.scroller().scrollTop);
  return Math.abs(after - g.before) > 1;
}
const what = (page, w) => page.evaluate((w) => (window.__probe.what = w), w);

// ---- the scenarios. Each starts at the bottom of the chat, following it. 1-6 read up; 7 has nothing to read. ----
const SCENARIOS = {
  1: { name: 'wheel up: small steps (10-40 px), then big steps', run: async (page) => {
    await what(page, 'small');
    for (const dy of [-10, -20, -30, -40, -20, -10, -30, -40]) { await wheel(page, dy); await sleep(180); }
    await sleep(1500);
    await what(page, 'big');
    for (let i = 0; i < 4; i++) { await wheel(page, -400); await sleep(250); }
    await sleep(2000);
  } },
  2: { name: 'scroll up a long way, stop, wait 10 s', run: async (page) => {
    for (let i = 0; i < 10; i++) { await wheel(page, -400); await sleep(60); }
    await what(page, 'wait');
    await sleep(10000);
  } },
  3: { name: 'drag the scrollbar thumb up, then down', run: async (page) => {
    const up = await dragThumb(page, -60);
    await what(page, 'after-up');
    await sleep(1500);
    const down = await dragThumb(page, 25);
    await what(page, 'after-down');
    await sleep(1500);
    return up || down ? '' : 'thumb did not move the chat (no scrollbar here)';
  } },
  4: { name: 'reach the top so "load earlier" fires, keep scrolling', run: async (page) => {
    const first = () => page.evaluate(() => document.querySelector('[data-message-id]')?.getAttribute('data-message-id'));
    const f0 = await first();
    let loaded = 0;
    for (let i = 0; i < 160 && loaded < 2; i++) {
      await wheel(page, -600);
      await sleep(90);
      if ((await first()) !== f0 && loaded === 0) loaded = 1;
      if (loaded === 1) { await what(page, 'after-load'); for (let k = 0; k < 8; k++) { await wheel(page, -150); await sleep(200); } loaded = 2; }
    }
    await sleep(2500);
    return loaded ? '' : 'earlier page never loaded';
  } },
  5: { name: 'reading mid-chat: a message lands, an approval and a question card arrive, the strip hides', run: async (page) => {
    for (let i = 0; i < 4; i++) { await wheel(page, -400); await sleep(80); }
    await sleep(800);
    await what(page, 'message');
    await page.evaluate(() => window.__harness.message());
    await sleep(1500);
    await what(page, 'approval');
    const ap = await page.evaluate(() => window.__harness.approval());
    await sleep(1500);
    await what(page, 'question');
    await page.evaluate(() => window.__harness.message('question'));
    await sleep(1500);
    await what(page, 'withdraw');
    const cards = await page.evaluate(() => ({ approval: document.querySelectorAll('[data-approval-card]').length, question: document.querySelectorAll('[data-question-card="open"]').length }));
    await page.evaluate((id) => window.__harness.withdraw(id), ap);
    await sleep(2000);
    return cards.approval && cards.question ? '' : `cards missing: ${JSON.stringify(cards)}`;
  } },
  6: { name: 'phone: touch-scroll up, keyboard opens and closes; desktop: the composer grows', run: async (page, { engine, phone }) => {
    if (phone) {
      await what(page, 'touch');
      await touchScroll(page, engine, -300);
      await sleep(1200);
      await touchScroll(page, engine, -60);
      await sleep(1200);
      await what(page, 'keyboard');
      const vp = page.viewportSize();
      await page.evaluate(() => document.querySelector('textarea')?.focus({ preventScroll: true }));
      await page.setViewportSize({ width: vp.width, height: vp.height - 330 });
      await sleep(1500);
      await page.evaluate(() => document.activeElement?.blur());
      await page.setViewportSize(vp);
      await sleep(1500);
    } else {
      for (let i = 0; i < 4; i++) { await wheel(page, -400); await sleep(80); }
      await sleep(600);
    }
    await what(page, 'composer');
    await page.fill('textarea', 'line one\nline two\nline three\nline four\nline five\nline six');
    await sleep(1500);
    await page.fill('textarea', '');
    await sleep(1500);
  } },
  // F1 (Rigel): on a chat that fits the screen, an up-input scrolls nothing and makes no scroll event, so if it
  // stops following nothing turns it back on and the next reply streams out of view.
  7: { name: 'short chat: wheel up, a finger drag down, PageUp and Home scroll nothing; the reply is still followed', n: 1, follows: true, run: async (page, { engine, phone, streaming }) => {
    const over = () => page.evaluate(() => { const sc = window.__probe.scroller(); return Math.round(sc.scrollHeight - sc.clientHeight); });
    if ((await over()) > 0) return 'not short: the chat already scrolls';
    // Streaming: the first reply has just started when the person swipes (the probe's long pre-stream would
    // overflow a phone before the inputs land). Still: the reply starts after them.
    // Slow enough (25 a second) that WebKit's slower inputs still land on a short chat.
    const done = streaming ? page.evaluate(() => window.__harness.stream(400, 25)) : null;
    await what(page, 'wheel');
    await wheel(page, -200);
    await sleep(200);
    if (phone) { await what(page, 'touch'); await touchScroll(page, engine, -100); await sleep(200); }
    await what(page, 'keys');
    await page.evaluate(() => { const sc = window.__probe.scroller(); sc.tabIndex = -1; sc.focus({ preventScroll: true }); });
    await page.keyboard.press('PageUp');
    await page.keyboard.press('Home');
    await sleep(200);
    if (streaming && (await over()) > 0) return 'not short: the reply passed the fold before the inputs ended';
    // From here the chat should follow the reply down: those moves are the point, not jumps.
    await page.evaluate(() => (window.__probe.on = false));
    await what(page, 'reply');
    await (done ?? page.evaluate(() => window.__harness.stream(400, 80)));
    await sleep(500);
    const g = await page.evaluate(() => { const sc = window.__probe.scroller(); return { gap: Math.round(sc.scrollHeight - sc.scrollTop - sc.clientHeight), over: Math.round(sc.scrollHeight - sc.clientHeight) }; });
    if (g.over <= 0) return 'reply never went past the fold';
    return g.gap <= 2 ? '' : `not followed: the reply ran ${g.gap} px below the screen`;
  } },
};

const runs = [];
const which = SCEN === 'all' ? Object.keys(SCENARIOS) : SCEN.split(',');
for (const engine of ENGINES) {
  for (const width of WIDTHS) {
    for (const streaming of STREAMS) {
      for (const k of which) {
        const sc = SCENARIOS[k];
        const { browser, page, phone, errors } = await open(engine, width, sc.n);
        let note = '';
        try {
          if (streaming && !sc.n) {
            await page.evaluate(() => void window.__harness.stream(100000, 50));
            await sleep(1200);
          }
          await page.evaluate(() => Object.assign(window.__probe, { on: true, jumps: [], frames: 0, last: null, anchor: null, what: 'start' }));
          let strip = null;
          if (STRIP) strip = shoot(page, STRIP.replace(/\.png$/, `-${engine}-${width}-s${k}-${streaming ? 'stream' : 'still'}`));
          note = (await sc.run(page, { engine, phone, streaming })) || '';
          if (strip) await strip;
        } catch (e) {
          note = `error: ${String(e).split('\n')[0]}`;
        }
        const p = await page.evaluate(() => { const sc = window.__probe.scroller(); return { jumps: window.__probe.jumps, frames: window.__probe.frames, gap: Math.round(sc.scrollHeight - sc.scrollTop - sc.clientHeight) }; });
        // Every scenario ends away from the bottom (the person read up); 0 means the chat pulled them back down.
        if (p.gap < 2 && !sc.follows) note = `${note ? `${note}; ` : ''}ended at the bottom: the chat pulled the reader back`;
        await browser.close();
        const maxPx = p.jumps.reduce((m, j) => Math.max(m, Math.abs(j.dy)), 0);
        const r = { engine, width, scenario: Number(k), streaming, jumps: p.jumps.length, maxPx, frames: p.frames, note, errors: errors.slice(0, 3), first: p.jumps.slice(0, 6) };
        runs.push(r);
        console.log(JSON.stringify(r));
      }
    }
  }
}
server.close();
if (JSON_OUT) fs.writeFileSync(JSON_OUT, JSON.stringify(runs, null, 1));
const total = runs.reduce((s, r) => s + r.jumps + (/pulled|never loaded|not followed|never went past|not short/.test(r.note) ? 1 : 0), 0);
console.log(`total jumps: ${total} in ${runs.length} runs`);
process.exit(total ? 1 : 0);

// A frame strip: 8 shots of the chat 250 ms apart from the first input on, tiled side by side (needs ffmpeg).
async function shoot(page, stem) {
  const files = [];
  await sleep(150);
  for (let i = 0; i < 8; i++) {
    const f = `${stem}-${i}.png`;
    await page.screenshot({ path: f });
    files.push(f);
    await sleep(250);
  }
  try {
    execFileSync('ffmpeg', ['-y', '-loglevel', 'error', ...files.flatMap((f) => ['-i', f]), '-filter_complex', `${files.map((_, i) => `[${i}]scale=iw/2:-1[s${i}]`).join(';')};${files.map((_, i) => `[s${i}]`).join('')}hstack=inputs=${files.length}`, `${stem}.png`]);
    files.forEach((f) => fs.unlinkSync(f));
  } catch {
    // the separate shots stay
  }
}
