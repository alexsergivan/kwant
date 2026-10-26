// Headless phone check of the CAPTCHA hand-off against tests/captcha_ui_harness.rs:
// open the bot's chat, tap Solve it, check the view zooms to the puzzle, drag the slider with a
// finger, and check the step closes by itself and the full-screen view goes away.
// node captcha-handoff.mjs <harness dir> <out dir>
// Needs playwright-core (PW_CORE=/path/to/playwright-core/index.mjs) and Chromium (CHROMIUM=...).
const { chromium } = await import(process.env.PW_CORE || 'playwright-core');
import { existsSync, readFileSync, writeFileSync } from 'node:fs';

const [dir, out] = process.argv.slice(2);
const ready = JSON.parse(readFileSync(`${dir}/ready.json`, 'utf8'));
const base = ready.url.replace(/\/$/, '');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const steps = async () => (await (await fetch(`${base}/api/human-steps`)).json()).filter((s) => s.kind === 'captcha');
const openStep = async () => (await steps()).find((s) => s.status === 'open');
const results = [];

async function waitFor(fn, what, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await fn();
    if (v) return v;
    await sleep(200);
  }
  throw new Error(`timed out: ${what}`);
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || '/usr/bin/chromium', args: ['--no-sandbox'] });
try {
  for (const [i, vp] of [{ width: 390, height: 844 }, { width: 412, height: 915 }].entries()) {
    const tag = `${vp.width}x${vp.height}`;
    if (i > 0) writeFileSync(`${dir}/rearm-${i}`, '');
    const step = await waitFor(openStep, 'an open captcha step');
    const ctx = await browser.newContext({ viewport: vp, isMobile: true, hasTouch: true, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await page.goto(base + '/');
    await page.getByText(ready.bot_name, { exact: true }).first().tap();
    const solve = page.getByRole('button', { name: 'Solve it' }).last();
    await solve.waitFor({ timeout: 15000 });
    await solve.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${out}/${tag}-1-card.png` });
    await solve.tap();

    const overlay = page.locator('[data-browser-fullscreen]');
    await overlay.waitFor({ timeout: 15000 });
    const img = overlay.locator('[data-browser-stage] img');
    await img.waitFor({ timeout: 15000 });
    await page.getByText('continues by itself').first().waitFor();
    await sleep(800);
    const geo = await page.evaluate(() => {
      const stage = document.querySelector('[data-browser-fullscreen] [data-browser-stage]');
      const im = stage.querySelector('img');
      const m = /translate\(([-\d.]+)px, ([-\d.]+)px\) scale\(([-\d.]+)\)/.exec(im.style.transform);
      const r = stage.getBoundingClientRect();
      return { tx: +m[1], ty: +m[2], scale: +m[3], left: r.left, top: r.top, width: r.width, height: r.height };
    });
    const fit = geo.width / 1280;
    const box = step.target;
    const boxLeft = box.x * geo.scale + geo.tx;
    const boxRight = (box.x + box.width) * geo.scale + geo.tx;
    const zoomed = geo.scale > fit * 2 && boxLeft >= -1 && boxRight <= geo.width + 1;
    if (!zoomed) throw new Error(`not zoomed to the puzzle: ${JSON.stringify({ geo, fit, box })}`);
    await page.screenshot({ path: `${out}/${tag}-2-zoomed.png` });

    // The slider knob sits at page (510, 500); drag it 260 page px right with one finger
    const toScreen = (x, y) => ({ x: geo.left + x * geo.scale + geo.tx, y: geo.top + y * geo.scale + geo.ty });
    const from = toScreen(510, 500);
    const to = toScreen(770, 500);
    const cdp = await ctx.newCDPSession(page);
    const touch = (type, p) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: p ? [{ x: p.x, y: p.y, id: 1 }] : [] });
    await touch('touchStart', from);
    const n = 24;
    for (let k = 1; k <= n; k++) {
      await touch('touchMove', { x: from.x + ((to.x - from.x) * k) / n, y: from.y });
      await sleep(20);
      if (k === n / 2) await page.screenshot({ path: `${out}/${tag}-3-dragging.png` });
    }
    await touch('touchEnd');

    const closed = await waitFor(async () => (await steps()).find((s) => s.id === step.id && s.status !== 'open'), 'the step to close', 20000);
    if (closed.status !== 'resolved_auto') throw new Error(`step closed as ${closed.status}`);
    await page.getByText(/Solved, .* continues/).first().waitFor({ timeout: 5000 });
    await page.screenshot({ path: `${out}/${tag}-4-solved.png` });
    await overlay.waitFor({ state: 'detached', timeout: 10000 });
    await page.getByText(/Solved\. .* continues\./).first().waitFor({ timeout: 5000 });
    await page.screenshot({ path: `${out}/${tag}-5-card-after.png` });
    if (errors.length) throw new Error(`page errors: ${errors.join('; ')}`);
    results.push({ viewport: tag, step: step.id, zoom: +(geo.scale / fit).toFixed(2), status: closed.status });
    console.log(`PASS ${tag}: zoom ${(geo.scale / fit).toFixed(2)}x of fit, drag solved it, step ${closed.status}, view closed`);
    await ctx.close();
  }
} finally {
  await browser.close();
}
writeFileSync(`${out}/playwright-result.json`, JSON.stringify(results, null, 2));
