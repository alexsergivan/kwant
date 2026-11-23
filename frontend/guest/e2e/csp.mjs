// pa5 guest page in a real Chrome under exactly pa1's pocket CSP (guest/fake/views.mjs CSP), from the fake guest port.
// Fails on: any securitypolicyviolation or CSP console error, a page error, any request that is not same-origin,
// a missing or different CSP header, sideways scrolling at 390 px, or a visible tap target under 44 px.
// Also writes a 390 px shot of every state and measures first paint on throttled networks (reported, not gated).
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/csp.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';
import { CSP, order } from '../fake/views.mjs';

const pw = await import(process.env.PW_CORE || 'playwright-core');
const out = path.resolve(process.argv[2] || 'guest-shots');
fs.mkdirSync(out, { recursive: true });

const fake = await startFake({ port: 0, actionDelayMs: 700 });
const boot = await startFake({ port: 0, boot: true });
const origin = `http://127.0.0.1:${fake.port}`;
const browser = await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox'] });

const problems = [];
const results = { csp: CSP, states: {}, timing: {} };

async function newPage({ throttle } = {}) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, locale: 'en-US', timezoneId: 'Europe/Ljubljana' });
  await ctx.addInitScript(() => {
    window.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(`${e.violatedDirective} ${e.blockedURI || ''} ${e.sourceFile || ''}:${e.lineNumber || ''}`.trim()));
    window.__shown = null;
    new MutationObserver(() => {
      const s = document.querySelector('[data-screen]');
      if (!window.__shown && s && s.getAttribute('data-screen') !== 'loading') window.__shown = performance.now();
    }).observe(document, { childList: true, subtree: true });
  });
  const page = await ctx.newPage();
  const log = [];
  page.on('console', (m) => {
    if (/Content.Security.Policy|Refused to/i.test(m.text())) log.push(`console: ${m.text()}`);
  });
  page.on('pageerror', (e) => log.push(`pageerror: ${e.message}`));
  page.on('request', (r) => {
    const u = r.url();
    if (!u.startsWith(`http://127.0.0.1:`) && !u.startsWith('ws://127.0.0.1:') && !u.startsWith('data:')) log.push(`third-party request: ${u}`);
  });
  if (throttle) {
    const cdp = await ctx.newCDPSession(page);
    await cdp.send('Network.enable');
    await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
    await cdp.send('Network.emulateNetworkConditions', { offline: false, ...throttle });
  }
  return { ctx, page, log };
}

async function check(name, page, log) {
  const v = await page.evaluate(() => window.__violations);
  for (const x of v) problems.push(`${name}: CSP violation: ${x}`);
  for (const x of log) problems.push(`${name}: ${x}`);
  const layout = await page.evaluate(() => {
    const small = [];
    for (const el of document.querySelectorAll('button, input:not(.vh), a, label.creature, label.team')) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height) continue;
      if (getComputedStyle(el).visibility === 'hidden') continue;
      if (r.height < 44 || r.width < 44) small.push(`${el.tagName.toLowerCase()}.${el.className} "${(el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 24)}" ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    return { scrollWidth: document.documentElement.scrollWidth, small };
  });
  if (layout.scrollWidth > 390) problems.push(`${name}: scrolls sideways at 390 (${layout.scrollWidth} px)`);
  for (const s of layout.small) problems.push(`${name}: tap target under 44 px: ${s}`);
}

async function shot(page, file) {
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(out, `${file}.png`), fullPage: true });
}

const ready = (page, screen) => page.waitForSelector(screen ? `[data-screen="${screen}"]` : '[data-screen]:not([data-screen="loading"])', { timeout: 15000 });

// The detector itself: a style element added after load must be reported, or the run proves nothing.
{
  const { ctx, page } = await newPage();
  const res = await page.goto(`${origin}/p/lobby`);
  await ready(page);
  await page.evaluate(() => {
    const s = document.createElement('style');
    s.textContent = 'body{outline:1px solid red}';
    document.head.append(s);
  });
  await page.waitForTimeout(200);
  const v = await page.evaluate(() => window.__violations);
  if (!v.some((x) => x.startsWith('style-src'))) problems.push('canary: an inline style element was not reported, so the CSP check is not working');
  if (res.headers()['content-security-policy'] !== CSP) problems.push(`canary: CSP header is "${res.headers()['content-security-policy']}"`);
  results.canary = v;
  await ctx.close();
}

for (const name of order) {
  const { ctx, page, log } = await newPage();
  const res = await page.goto(`${origin}/p/${name}`);
  if (res.headers()['content-security-policy'] !== CSP) problems.push(`${name}: wrong CSP header`);
  const t0 = Date.now();
  if (name === 'away') await page.waitForSelector('[data-screen="away"]', { timeout: 20000 });
  else await ready(page);
  const screen = await page.getAttribute('[data-screen]', 'data-screen');
  results.states[name] = { screen, ms: Date.now() - t0 };
  if (name === 'rejoin') await page.waitForSelector('.toast');
  if (name === 'picture') await page.waitForFunction(() => document.querySelector('.pic img')?.complete);
  await shot(page, name);

  if (name === 'join') {
    await page.click('button[data-join]');
    await page.waitForSelector('[data-err]:not(:empty)');
    await shot(page, 'join-error');
    await page.fill('#name', 'Mia');
    await page.click('label.creature:nth-child(3)');
    await page.click('label.team:has-text("Quizzly Bears")');
    await page.focus('#name');
    await shot(page, 'join-filled');
    await page.click('button[data-join]');
    await ready(page, 'lobby');
    await page.waitForSelector("text=You're in, Mia!");
    await shot(page, 'join-to-lobby');
    await page.keyboard.press('Tab'); // keyboard reachable: focus lands somewhere visible
  }
  if (name === 'choice') {
    await page.click('button[data-answer="B"]');
    await page.click('button[data-answer="C"]', { force: true, timeout: 2000 }).catch(() => {});
    await shot(page, 'choice-picked');
    await page.waitForSelector('text=Locked in');
    const sent = fake.log.filter((l) => l === 'POST /api/pocket/choice/action').length;
    if (sent !== 1) problems.push(`choice: ${sent} answers sent, first tap should be the only one`);
  }
  if (name === 'text') {
    await page.fill('input[data-answer]', 'Canbera');
    await shot(page, 'text-typed');
  }
  if (name === 'away') {
    await page.waitForSelector('text=tried 2 times', { timeout: 20000 });
    await shot(page, 'away');
  }
  await check(name, page, log);
  await ctx.close();
}

// Reduced motion: the page still works and the toast does not animate.
{
  const { ctx, page, log } = await newPage();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(`${origin}/p/rejoin`);
  await page.waitForSelector('.toast');
  const anim = await page.evaluate(() => getComputedStyle(document.querySelector('.toast')).animationName);
  if (anim !== 'none') problems.push(`reduced motion: toast animates (${anim})`);
  await check('reduced-motion', page, log);
  await ctx.close();
}

// First paint on throttled networks. DevTools "Slow 4G" (562.5 ms, 180 KB/s down) and Lighthouse's slow 4G
// (150 ms, 1.6 Mbps). Reported in pa5/impl.md, not gated: loopback plus CDP throttling is a model, not a phone.
const presets = {
  'devtools-slow-4g': { latency: 562.5, downloadThroughput: 180000, uploadThroughput: 84375 },
  'lighthouse-slow-4g': { latency: 150, downloadThroughput: 204800, uploadThroughput: 84375 },
};
for (const [label, throttle] of Object.entries(presets)) {
  for (const [how, base] of [['fetch-state', origin], ['boot-json', `http://127.0.0.1:${boot.port}`]]) {
    const runs = [];
    for (let i = 0; i < 3; i++) {
      const { ctx, page } = await newPage({ throttle });
      await page.goto(`${base}/p/join`);
      try {
        await page.waitForSelector('[data-screen="join"]', { timeout: 30000 });
      } catch {
        const shown = await page.getAttribute('[data-screen]', 'data-screen').catch(() => null);
        problems.push(`timing ${label} ${how}: no join screen in 30 s (showing ${shown})`);
        await page.screenshot({ path: path.join(out, `timing-fail-${label}-${how}.png`) }).catch(() => {});
        await ctx.close();
        continue;
      }
      runs.push(
        await page.evaluate(() => ({
          fcp: Math.round(performance.getEntriesByName('first-contentful-paint')[0]?.startTime ?? -1),
          join: Math.round(window.__shown ?? -1),
        })),
      );
      await ctx.close();
    }
    const med = (k) => runs.map((r) => r[k]).sort((a, b) => a - b)[Math.floor(runs.length / 2)];
    if (!runs.length) continue;
    results.timing[`${label} ${how}`] = { fcp_ms: med('fcp'), join_screen_ms: med('join'), runs };
  }
}

await browser.close();
await fake.close();
await boot.close();
results.problems = problems;
fs.writeFileSync(path.join(out, 'results.json'), JSON.stringify(results, null, 2));
console.log(JSON.stringify({ states: Object.fromEntries(Object.entries(results.states).map(([k, v]) => [k, v.screen])), timing: Object.fromEntries(Object.entries(results.timing).map(([k, v]) => [k, `${v.fcp_ms} ms first paint, ${v.join_screen_ms} ms join screen`])) }, null, 2));
if (problems.length) {
  console.error(`\n${problems.length} problem(s):\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log(`\nguest page clean under the pocket CSP; ${fs.readdirSync(out).filter((f) => f.endsWith('.png')).length} shots in ${out}`);
