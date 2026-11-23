// pa5 guest page in a real Chrome under exactly pa1's pocket CSP (guest/fake/views.mjs CSP), from the fake guest port.
// Fails on: any securitypolicyviolation or CSP console error, a page error, any request that is not same-origin,
// a missing or different CSP header, sideways scrolling at 390 px, or a visible tap target under 44 px.
// Also walks Secret Santa's G1-G11 (pa10a) from the fake santa box. Writes a 390 px shot of every state and measures first paint on throttled networks (reported, not gated).
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/csp.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';
import { CSP, order } from '../fake/views.mjs';
import { santaScenarios } from '../fake/santa.mjs';

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
    // Pixel icons draw in the text colour; a rect with no fill would render black on the dark page.
    // A fill that isn't currentColor or #rrggbb ("undefined", a name, empty) draws black too.
    const unfilled = [...document.querySelectorAll('svg rect')].filter((r) => {
      const f = r.getAttribute('fill');
      return f !== 'currentColor' && !/^#[0-9a-f]{6}$/i.test(f ?? '');
    }).length;
    return { scrollWidth: document.documentElement.scrollWidth, small, unfilled };
  });
  if (layout.scrollWidth > 390) problems.push(`${name}: scrolls sideways at 390 (${layout.scrollWidth} px)`);
  if (layout.unfilled) problems.push(`${name}: ${layout.unfilled} svg rect(s) whose fill isn't currentColor or #rrggbb (would draw black)`);
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

// Removed and away load as pa1 serves them, with the public facts in the page, so the bar keeps the title (fix 5).
const bootOrigin = `http://127.0.0.1:${boot.port}`;
for (const name of order) {
  const { ctx, page, log } = await newPage();
  const res = await page.goto(`${name === 'removed' || name === 'away' ? bootOrigin : origin}/p/${name}`);
  if (res.headers()['content-security-policy'] !== CSP) problems.push(`${name}: wrong CSP header`);
  const t0 = Date.now();
  if (name === 'away') await page.waitForSelector('[data-screen="away"]', { timeout: 20000 });
  else await ready(page);
  const screen = await page.getAttribute('[data-screen]', 'data-screen');
  results.states[name] = { screen, ms: Date.now() - t0 };
  if (name === 'rejoin') await page.waitForSelector('.toast');
  if (name === 'picture') await page.waitForFunction(() => document.querySelector('.pic img')?.complete);
  // A seated load mid-game says "You're back" for 4 s. Only the rejoin shot keeps it (at its bottom place), so the
  // other shots show the meta row and the timer it used to cover. Removing a node is not a style change.
  if (name !== 'rejoin') await page.evaluate(() => document.querySelector('.toast')?.remove());
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

// Away mid-game (fix 5): the box drops while a question is up. The last view stays, so the bar keeps the title and
// the score, and the lead says the score is safe. The phone "comes back online" to make the page notice at once.
{
  const { ctx, page, log } = await newPage();
  await page.goto(`${origin}/p/rejoin`);
  await ready(page, 'question');
  await page.evaluate(() => document.querySelector('.toast')?.remove());
  await page.route('**/api/pocket/**', (r) => r.abort('connectionfailed'));
  await page.evaluate(() => window.dispatchEvent(new Event('online')));
  await page.waitForSelector('[data-screen="away"]', { timeout: 10000 });
  const bar = await page.textContent('header.bar');
  const lead = await page.textContent('main');
  if (!bar.includes('Friday Pub Quiz') || !bar.includes('1,240')) problems.push(`away mid-game: bar is "${bar.trim()}"`);
  if (!lead.includes('Your score is safe: 1,240.')) problems.push('away mid-game: no "Your score is safe: 1,240."');
  await shot(page, 'away-midgame');
  results.states['away-midgame'] = { screen: 'away' };
  await page.unroute('**/api/pocket/**');
  await check('away-midgame', page, log.filter((l) => !/ERR_CONNECTION|Failed to load resource|net::/.test(l)));
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

// pa10a Secret Santa, G1-G11 (guest-brief.md), from the fake santa box (fake/santa.mjs) in Lin's server-impl shapes.
// Same checks as the quiz screens; shots named santa-g<N>-<what>. The kit comes from the public facts, so the Santa
// chunk loads lazily, and the walk fails if the quiz screens show instead.
{
  const santaShot = async (page, log, name, file = name) => {
    await page.evaluate(() => document.querySelector('.toast')?.remove());
    await shot(page, file);
    await check(file, page, log);
    results.states[file] = { screen: await page.getAttribute('[data-screen]', 'data-screen') };
  };
  const go = async (code, screen, { base = origin, reduced = false } = {}) => {
    const p = await newPage();
    if (reduced) await p.page.emulateMedia({ reducedMotion: 'reduce' });
    // A pre-seated scenario (state is a seat, not a status) is a guest who joined earlier: give the page its seat
    // cookie, as pa1 would have set it on join. Without it the box answers 401 and the page rightly shows G10 late.
    if (typeof santaScenarios[code]?.state === 'object') {
      await p.ctx.addCookies([{ name: `kp_${code}`, value: 'fake', domain: '127.0.0.1', path: `/api/pocket/${code}`, httpOnly: true, sameSite: 'Strict' }]);
    }
    const res = await p.page.goto(`${base}/p/${code}`);
    if (res.headers()['content-security-policy'] !== CSP) problems.push(`${code}: wrong CSP header`);
    await p.page.waitForSelector(`main.sa[data-screen="${screen}"]`, { timeout: 20000 });
    return p;
  };

  // G1 join, then G2 waiting with the wish saved
  {
    const { ctx, page, log } = await go('santa-join', 'join');
    await santaShot(page, log, 'santa-g1-join');
    await page.fill('#s-name', 'Ana');
    await page.click('label.creature:nth-child(3)');
    await page.fill('#s-pin', '1234');
    const dots = await page.evaluate(() => [...document.querySelectorAll('.pins .pin')].map((b) => b.textContent).join(''));
    if (dots !== '••••') problems.push(`santa-g1: PIN boxes show "${dots}", not four dots`);
    await page.fill('#s-wish', 'Warm socks\nA good paperback');
    await santaShot(page, log, 'santa-g1-join', 'santa-g1-join-filled');
    await page.click('button[data-join]');
    await page.waitForSelector('main.sa[data-screen="waiting"]');
    await page.waitForSelector('text=Edit my wish list');
    await santaShot(page, log, 'santa-g2-waiting');
    await ctx.close();
  }
  // G3 rejoin
  {
    const { ctx, page, log } = await go('santa-join', 'join');
    await page.click('button[data-to-rejoin]');
    await page.waitForSelector('main.sa[data-screen="rejoin"]');
    await santaShot(page, log, 'santa-g3-rejoin');
    await ctx.close();
  }
  // G4 wrong PIN, the tries from the server
  {
    const { ctx, page, log } = await go('santa-wrongpin', 'join');
    await page.click('button[data-to-rejoin]');
    await page.fill('#s-bname', 'Mia');
    await page.fill('#s-bpin', '0000');
    await page.click('button[data-rejoin]');
    await page.waitForSelector('[data-problem]');
    const p = await page.textContent('[data-problem]');
    if (!p.includes('3 more tries')) problems.push(`santa-g4: problem says "${p}"`);
    await santaShot(page, log, 'santa-g4-wrong-pin');
    await ctx.close();
  }
  // G5 unwrap, G6 match (lid lifts, card pops), G7 Santa asks
  {
    const { ctx, page, log } = await go('santa-unwrap', 'unwrap');
    await santaShot(page, log, 'santa-g5-unwrap');
    const lid = await page.evaluate(() => getComputedStyle(document.querySelector('.giftbox .lid')).transitionDuration);
    if (!lid.includes('0.18s')) problems.push(`santa-g5: lid transition is ${lid}, not 180 ms`);
    await page.click('button[data-open]');
    await page.waitForSelector('main.sa[data-screen="match"]', { timeout: 10000 });
    await page.waitForSelector('text=A good paperback, any thriller');
    await santaShot(page, log, 'santa-g6-match');
    const remembered = await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('kp-santa-open:')).length);
    if (remembered !== 1) problems.push('santa-g6: the unwrap is not remembered on this device');
    if (await page.$('.kw-sticker-holo')) problems.push('santa-g7: holo on a guest screen');
    if (!(await page.$('[data-sticker="ink"]'))) problems.push('santa-g7: no ink New sticker for the new question');
    await page.click('button[data-chip]');
    await page.locator('#asks-out').scrollIntoViewIfNeeded();
    await santaShot(page, log, 'santa-g7-asks');
    await page.click('button[data-send-ask]');
    await page.waitForSelector("text=Waiting for Jonas's answer");
    await santaShot(page, log, 'santa-g7-asks', 'santa-g7-asked');
    // A reload on the same device skips the box.
    await page.reload();
    await page.waitForSelector('main.sa[data-screen="match"]', { timeout: 10000 });
    await ctx.close();
  }
  // G5 after match_changed: closed until Open it again
  {
    const { ctx, page, log } = await go('santa-changed', 'unwrap');
    await page.waitForSelector('[data-changed]');
    await santaShot(page, log, 'santa-g5-match-changed');
    await ctx.close();
  }
  // G8 reveal, G10 late, G11 ended
  for (const [code, screen, file] of [['santa-reveal', 'reveal', 'santa-g8-reveal'], ['santa-late', 'late', 'santa-g10-late'], ['santa-ended', 'ended', 'santa-g11-ended']]) {
    const { ctx, page, log } = await go(code, screen);
    await santaShot(page, log, file);
    await ctx.close();
  }
  // G9 away (served with the boot facts, as pa1 does)
  {
    const { ctx, page, log } = await go('santa-away', 'away', { base: bootOrigin });
    await santaShot(page, log, 'santa-g9-away');
    await ctx.close();
  }
  // Reduced motion: the box opens with no lid transition and no pop
  {
    const { ctx, page, log } = await go('santa-unwrap', 'unwrap', { reduced: true });
    const lid = await page.evaluate(() => getComputedStyle(document.querySelector('.giftbox .lid')).transitionDuration);
    if (lid !== '0s') problems.push(`santa reduced motion: lid transition ${lid}`);
    await page.click('button[data-open]');
    await page.waitForSelector('main.sa[data-screen="match"]', { timeout: 10000 });
    const anim = await page.evaluate(() => getComputedStyle(document.querySelector('.matchcard')).animationName);
    if (anim !== 'none') problems.push(`santa reduced motion: match card animates (${anim})`);
    await check('santa-reduced-motion', page, log);
    await ctx.close();
  }
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
