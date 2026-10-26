// Desktop: Solve it opens the side panel (no full screen) and a mouse drag on the panel image solves the slider.
const { chromium } = await import(process.env.PW_CORE || 'playwright-core');
import { readFileSync, writeFileSync } from 'node:fs';
const [dir, out] = process.argv.slice(2);
const ready = JSON.parse(readFileSync(`${dir}/ready.json`, 'utf8'));
const base = ready.url.replace(/\/$/, '');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const steps = async () => (await (await fetch(`${base}/api/human-steps`)).json()).filter((s) => s.kind === 'captcha');
async function waitFor(fn, what, ms = 20000) { const end = Date.now() + ms; while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(200); } throw new Error(`timed out: ${what}`); }
writeFileSync(`${dir}/rearm-desktop-${Date.now()}`, '');
const step = await waitFor(async () => (await steps()).find((s) => s.status === 'open'), 'open step');
const b = await chromium.launch({ executablePath: process.env.CHROMIUM || '/usr/bin/chromium', args: ['--no-sandbox'] });
const page = await (await b.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
await page.goto(base + '/');
await page.getByText(ready.bot_name, { exact: true }).first().click();
await page.getByRole('button', { name: 'Solve it' }).first().click();
await page.getByText('Controlling').first().waitFor({ timeout: 15000 });
if (await page.locator('[data-browser-fullscreen]').count()) throw new Error('full screen opened on desktop');
const img = page.locator('img[alt="Browser Viewport"]');
await img.waitFor();
await sleep(800);
await page.screenshot({ path: `${out}/1280x800-1-panel.png` });
const r = await img.boundingBox();
const s = Math.min(r.width / 1280, r.height / 800);
const ox = r.x + (r.width - 1280 * s) / 2, oy = r.y + (r.height - 800 * s) / 2;
await page.mouse.move(ox + 510 * s, oy + 500 * s);
await page.mouse.down();
for (let k = 1; k <= 20; k++) { await page.mouse.move(ox + (510 + 13 * k) * s, oy + 500 * s); await sleep(20); }
await page.mouse.up();
const closed = await waitFor(async () => (await steps()).find((x) => x.id === step.id && x.status !== 'open'), 'close');
await page.getByText(/Solved\. .* continues\./).first().waitFor({ timeout: 5000 });
await page.screenshot({ path: `${out}/1280x800-2-solved.png` });
console.log(`PASS 1280x800 mouse drag: step ${closed.status}`);
await b.close();
