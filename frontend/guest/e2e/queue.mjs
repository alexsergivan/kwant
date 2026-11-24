// pa11b the Queue ticket in a real Chrome under exactly pa1's pocket CSP, from the fake queue box (fake/queue.mjs) in
// Vega's queue.rs shapes. The guest page's 12 states (take, ticket, next, turn, skipped, served, leave, paused, closed,
// ended/unknown and the refusals 409 full, 409 paused, 410 closed, 429 phone, 429 network) at 390 px, a few at 1440,
// and the big screen's S1 open, S2 paused and Closed at 1920x1080. Fails on: a CSP violation or page error, a request
// that is not same-origin, sideways scrolling at 390, a visible tap target under 44 px, a name on screen that isn't
// this phone's, a cue that fires twice, the screen calling a host write.
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/queue.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';
import { CSP } from '../fake/views.mjs';

const pw = await import(process.env.PW_CORE || 'playwright-core');
const out = path.resolve(process.argv[2] || 'guest-shots');
fs.mkdirSync(out, { recursive: true });
const fake = await startFake({ port: 0, actionDelayMs: 300 });
const origin = `http://127.0.0.1:${fake.port}`;
const browser = await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const problems = [];
const results = { states: {}, screen: {} };
const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';

async function newPage({ width = 390, height = 844, reduced = false, ua, ios = false } = {}) {
  const mobile = width < 800;
  const ctx = await browser.newContext({
    viewport: { width, height }, deviceScaleFactor: mobile ? 2 : 1, isMobile: mobile, hasTouch: mobile, locale: 'en-US', timezoneId: 'Europe/Ljubljana',
    reducedMotion: reduced ? 'reduce' : 'no-preference', ...(ua ? { userAgent: ua } : {}),
  });
  await ctx.addInitScript((ios) => {
    window.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(`${e.violatedDirective} ${e.blockedURI || ''}`.trim()));
    // Every vibrate the page asks for, so the walk can count cues.
    window.__vib = [];
    // Safari on iPhone has no navigator.vibrate at all.
    try {
      if (ios) delete Navigator.prototype.vibrate;
      else Object.defineProperty(Navigator.prototype, 'vibrate', { configurable: true, value: (p) => (window.__vib.push(JSON.stringify(p)), true) });
    } catch {}
  }, ios);
  const page = await ctx.newPage();
  const log = [];
  page.on('console', (m) => {
    if (/Content.Security.Policy|Refused to/i.test(m.text())) log.push(`console: ${m.text()}`);
  });
  page.on('pageerror', (e) => log.push(`pageerror: ${e.message}`));
  page.on('request', (r) => {
    const u = r.url();
    if (!u.startsWith('http://127.0.0.1:') && !u.startsWith('ws://127.0.0.1:') && !u.startsWith('data:')) log.push(`third-party request: ${u}`);
  });
  return { ctx, page, log };
}

async function check(name, page, log, width = 390) {
  for (const x of await page.evaluate(() => window.__violations)) problems.push(`${name}: CSP violation: ${x}`);
  for (const x of log.splice(0)) problems.push(`${name}: ${x}`);
  const layout = await page.evaluate(() => {
    const small = [];
    for (const el of document.querySelectorAll('button, input:not(.vh), a')) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden') continue;
      if (r.height < 44 || r.width < 44) small.push(`${el.tagName.toLowerCase()}.${el.className} "${(el.textContent || '').trim().slice(0, 24)}" ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    const take = document.querySelector('[data-take]');
    const unfilled = [...document.querySelectorAll('svg rect')].filter((r) => {
      const f = r.getAttribute('fill');
      return f !== 'currentColor' && !/^#[0-9a-f]{6}$/i.test(f ?? '');
    }).length;
    return { scrollWidth: document.documentElement.scrollWidth, small, take: take ? take.getBoundingClientRect().height : null, unfilled };
  });
  if (layout.scrollWidth > width) problems.push(`${name}: scrolls sideways at ${width} (${layout.scrollWidth} px)`);
  for (const s of layout.small) problems.push(`${name}: tap target under 44 px: ${s}`);
  if (layout.take !== null && layout.take < 60) problems.push(`${name}: Take a ticket is ${layout.take} px, not 60`);
  if (layout.unfilled) problems.push(`${name}: ${layout.unfilled} svg rect(s) without a fill`);
  const txt = await page.evaluate(() => document.body.innerText);
  if (/undefined|null|NaN|\[object/.test(txt)) problems.push(`${name}: internal words on screen`);
  // Never anyone else's name: the fake's other customers have none, the host's name is Tine.
  if (/\b(Mia|Ben)\b/.test(txt)) problems.push(`${name}: another customer's name on screen`);
}

async function shot(page, log, file, { width = 390, full = true } = {}) {
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(out, `${file}.png`), fullPage: full });
  await check(file, page, log, width);
  results.states[file] = await page.getAttribute('[data-screen]', 'data-screen').catch(() => null);
}

let fresh = 0;
const go = async (scene, screen, opts = {}) => {
  const p = await newPage(opts);
  const code = `queue-${scene}--${++fresh}`;
  const res = await p.page.goto(`${origin}/p/${code}`);
  if (res.headers()['content-security-policy'] !== CSP) problems.push(`${code}: wrong CSP header`);
  try {
    await p.page.waitForSelector(`main.qg[data-screen="${screen}"]`, { timeout: 15000 });
  } catch {
    problems.push(`${code}: never showed ${screen} (showing ${await p.page.getAttribute('[data-screen]', 'data-screen').catch(() => '?')})`);
  }
  return { ...p, code };
};
const want = async (page, sel, what, timeout = 8000) => {
  try {
    await page.waitForSelector(sel, { timeout });
    return true;
  } catch {
    problems.push(`${what} (${sel}) not shown`);
    return false;
  }
};
const text = (page) => page.evaluate(() => document.body.innerText);

// 1 take, then Take a ticket: join {} first (no name), then take; the ticket is #28, 6th.
{
  const { ctx, page, log, code } = await go('take', 'take');
  await shot(page, log, 'queue-g01-take');
  if (!/You'd be/.test(await text(page))) problems.push('g01: no "You\'d be" line');
  await page.click('[data-take]');
  await want(page, 'main.qg[data-screen="waiting"]', 'g01 → ticket after Take');
  const t = await text(page);
  if (!/#28/.test(t) || !/6th/.test(t)) problems.push(`g01: the new ticket isn't #28, 6th`);
  const takes = fake.box(code).takes;
  if (takes.length !== 1 || Object.keys(takes[0]).length) problems.push(`g01: take sent ${JSON.stringify(takes)} (want one {} with names off)`);
  if (!(await page.title()).startsWith('(6th)')) problems.push(`g01: tab title "${await page.title()}"`);
  if (await page.$('.gnote.ok')) problems.push('g02: Welcome back right after taking');
  await shot(page, log, 'queue-g02-ticket');
  await ctx.close();
}
// 1 take with first names on: the field, and the name goes with take, never with join.
{
  const { ctx, page, log, code } = await go('take-name', 'take');
  await page.fill('#qname', 'Ana');
  await shot(page, log, 'queue-g01-take-name');
  await page.click('[data-take]');
  await want(page, 'main.qg[data-screen="waiting"]', 'g01 name → ticket');
  const takes = fake.box(code).takes;
  if (takes[0]?.name !== 'Ana') problems.push(`g01 name: take sent ${JSON.stringify(takes)}`);
  await ctx.close();
}
// 1 take before 3 calls today: no minutes anywhere.
{
  const { ctx, page, log } = await go('take-early', 'take');
  await shot(page, log, 'queue-g01-take-early');
  if (/\bmin(utes)?\b/.test(await text(page))) problems.push('g01 early: minutes shown before 3 calls');
  await ctx.close();
}
// 2 your ticket (Android), the buzz panel, Turn on sound; then calls move it to Next and to Your turn, one cue each.
{
  const { ctx, page, log, code } = await go('ticket', 'waiting');
  await want(page, '[data-ticket="waiting"]', 'g02 ticket');
  await want(page, '[data-sound]', 'g02 Turn on sound');
  await shot(page, log, 'queue-g02-ticket-came-back');
  await page.click('[data-sound]');
  await want(page, '.buzz .br:has-text("Chime") .st.ok', 'g02 Chime On after the tap');
  for (let i = 0; i < 3; i++) fake.push(code, (b) => b.callNext());
  await want(page, 'main.qg[data-screen="next"]', 'g04 next after 3 calls');
  await page.waitForTimeout(400);
  await shot(page, log, 'queue-g04-next-live');
  if (!(await page.title()).startsWith('(Next)')) problems.push(`g04: tab title "${await page.title()}"`);
  fake.push(code, (b) => b.callNext());
  await want(page, 'main.qg[data-screen="turn"]', 'g05 turn after the call');
  await page.waitForTimeout(1300);
  const vib = await page.evaluate(() => window.__vib);
  if (vib.length !== 2) problems.push(`cues: ${vib.length} vibrations (want 2: next once, turn once): ${vib.join(' ')}`);
  // A repeat of the same view (a socket reconnect) never cues again.
  fake.push(code, () => {});
  await page.waitForTimeout(500);
  if ((await page.evaluate(() => window.__vib.length)) !== 2) problems.push('cues: a repeated view cued again');
  const said = await page.evaluate(() => document.querySelector('[aria-live]')?.textContent ?? '');
  if (!/Your turn/.test(said)) problems.push(`g05: live region says "${said}"`);
  const fav = await page.evaluate(() => document.querySelector('link[rel="icon"][data-q]')?.getAttribute('href') ?? '');
  if (!fav.startsWith('data:image/svg+xml')) problems.push('g06: no ticket favicon while called');
  const titles = new Set();
  for (let i = 0; i < 6; i++) {
    titles.add(await page.title());
    await page.waitForTimeout(350);
  }
  if (!titles.has('#28') && !titles.has('#27')) problems.push(`g06: the tab title doesn't swap while called (${[...titles].join(' | ')})`);
  await ctx.close();
}
// 3 iPhone, came back: Welcome back, the iPhone honest line, Vibrate says it can't (and no vibrate row turned on).
{
  const { ctx, page, log } = await go('iphone', 'waiting', { ua: IPHONE, ios: true });
  await want(page, '.gnote.ok', 'g03 Welcome back');
  if (await page.$('.gnote.ok b')) problems.push('g03: Welcome back is bold');
  const t = await text(page);
  if (!/can't vibrate/.test(t)) problems.push('g03: no "iPhone can\'t vibrate" row');
  if (!/screen locks/.test(t)) problems.push('g03: not the iPhone honest line');
  await shot(page, log, 'queue-g03-iphone-back');
  await ctx.close();
}
// 4 next, 5 turn (role=timer counting down), 8 paused, 9 skipped, 10 served, 11 closed, 12 ended.
for (const [scene, screen, file, sel] of [
  ['next', 'next', 'queue-g04-next', '[data-ticket="next"]'],
  ['turn', 'turn', 'queue-g05-turn', '[role="timer"]'],
  ['paused', 'waiting', 'queue-g08-paused', '.gnote.warn'],
  ['skipped', 'skipped', 'queue-g09-skipped', '[data-take]'],
  ['served', 'served', 'queue-g10-served', '.gchip.ok'],
  ['left', 'left', 'queue-g07-left', '[data-take]'],
  ['closed', 'closed', 'queue-g11-closed', '[data-ticket="dim"]'],
  // No ticket on this phone: ended is the calm gone screen (data-screen missing, the same words).
  ['ended', 'missing', 'queue-g12-ended', 'h2'],
]) {
  const { ctx, page, log } = await go(scene, screen);
  await want(page, sel, file);
  await shot(page, log, file);
  // Welcome back only while plainly waiting in an open queue (Mira's ruling), and plain notes carry no bold.
  if (await page.$('.gnote.ok')) problems.push(`${file}: Welcome back shows on ${scene}`);
  if (scene === 'turn' && (await page.$$eval('.gnote:not(.warn) b', (x) => x.length))) problems.push('g05: the turn note is bold');
  if (scene === 'turn') {
    const a = await page.textContent('[data-cd] b');
    await page.waitForTimeout(2100);
    const b = await page.textContent('[data-cd] b');
    if (!/^\d:\d\d$/.test(a ?? '') || a === b) problems.push(`g05: the countdown doesn't count (${a} → ${b})`);
  }
  if (scene === 'skipped' && !/Now 6 waiting, about 18 minutes/.test(await text(page))) problems.push('g09: no "Now 6 waiting, about 18 minutes"');
  if (scene === 'paused' && !/Back in 10 minutes/.test(await text(page))) problems.push('g08: no pause text');
  if (scene === 'closed' && !/wasn't called before closing/.test(await text(page))) problems.push('g11: no closing line');
  await ctx.close();
}
// 7 leave: the inline confirm (alertdialog, focus on Stay), Escape stays, then Leave.
{
  const { ctx, page, log, code } = await go('ticket', 'waiting');
  await page.click('[data-leave-link]');
  await want(page, '[role="alertdialog"]', 'g07 confirm');
  const focused = await page.evaluate(() => document.activeElement?.hasAttribute('data-stay'));
  if (!focused) problems.push('g07: focus is not on Stay in line');
  await shot(page, log, 'queue-g07-leave-confirm');
  await page.keyboard.press('Escape');
  if (await page.$('[role="alertdialog"]')) problems.push('g07: Escape kept the confirm open');
  await page.click('[data-leave-link]');
  await page.click('[data-leave]');
  await want(page, 'main.qg[data-screen="left"]', 'g07 left after Leave');
  if (fake.box(code).ticket?.state !== 'left') problems.push('g07: the box ticket is not left');
  await ctx.close();
}
// Refusals on Take a ticket: 409 full, 409 paused, 410 closed, 429 phone, 429 network.
for (const [scene, file, re] of [
  ['full', 'queue-r-full', /queue is full right now/],
  ['r-paused', 'queue-r-paused', /Paused: “Back in 10 minutes”/],
  ['r-closed', 'queue-r-closed', /closed for today/],
  ['slow', 'queue-r-slow-phone', /This phone has taken 3 tickets/],
  ['slow-net', 'queue-r-slow-network', /from this network/],
]) {
  const { ctx, page, log } = await go(scene, 'take');
  await page.click('[data-take]');
  await page.waitForFunction((src) => new RegExp(src).test(document.body.innerText), re.source, { timeout: 8000 }).catch(() => problems.push(`${file}: no "${re.source}"`));
  if (!(await page.isDisabled('[data-take]'))) problems.push(`${file}: Take a ticket stays on`);
  await shot(page, log, file);
  await ctx.close();
}
// Reduced motion while called: the title holds still and nothing animates.
{
  const { ctx, page, log } = await go('turn', 'turn', { reduced: true });
  const titles = new Set();
  for (let i = 0; i < 5; i++) {
    titles.add(await page.title());
    await page.waitForTimeout(300);
  }
  if (titles.size !== 1) problems.push(`reduced motion: the tab title swaps (${[...titles].join(' | ')})`);
  const anim = await page.evaluate(() => [...document.querySelectorAll('main.qg *')].map((e) => getComputedStyle(e).animationName).filter((a) => a && a !== 'none'));
  if (anim.length) problems.push(`reduced motion: ${anim.length} element(s) animate`);
  await check('queue-reduced-motion', page, log);
  await ctx.close();
}
// 1440: the page stays one 480 px column.
for (const [scene, screen] of [['take', 'take'], ['ticket', 'waiting'], ['turn', 'turn']]) {
  const { ctx, page, log } = await go(scene, screen, { width: 1440, height: 900 });
  await shot(page, log, `queue-1440-${scene}`, { width: 1440 });
  await ctx.close();
}

// Big screen: S1 open, a new call swaps the number (and says it), S2 paused, Closed. Read-only: no host writes.
{
  const tvgo = async (scene, screen, opts = {}) => {
    const p = await newPage({ width: 1920, height: 1080, ...opts });
    const code = `screen-queue-${scene}--${++fresh}`;
    await p.page.goto(`${origin}/p/${code}#screen=tok-${scene}`);
    await p.page.waitForSelector(`.qtv[data-screen="${screen}"]`, { timeout: 15000 }).catch(() => problems.push(`${code}: never showed ${screen}`));
    return { ...p, code };
  };
  const tvShot = async (page, log, file, code) => {
    await page.waitForTimeout(400);
    for (const x of await page.evaluate(() => window.__violations)) problems.push(`${file}: CSP violation: ${x}`);
    for (const x of log.splice(0)) problems.push(`${file}: ${x}`);
    if (page.url().includes('#')) problems.push(`${file}: the token stays in the address bar`);
    const small = await page.evaluate(() => {
      const tv = document.querySelector('.tv');
      const s = Number(getComputedStyle(tv).getPropertyValue('--s')) || 1;
      return [...tv.querySelectorAll('*')].filter((el) => !el.closest('svg') && [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) && parseFloat(getComputedStyle(el).fontSize) < 32).map((el) => el.textContent.trim().slice(0, 30)).concat(s ? [] : ['no scale']);
    });
    for (const x of small) problems.push(`${file}: text under 32 px: ${x}`);
    const wide = await page.evaluate(() => [...document.querySelectorAll('.joinq b, .joinq i')].filter((e) => e.scrollWidth > e.clientWidth + 0.5 || e.getClientRects().length > 1).map((e) => e.textContent));
    for (const x of wide) problems.push(`${file}: join box line overflows or wraps: ${x}`);
    if (/\b(Mia|Ben|Ana)\b/.test(await text(page))) problems.push(`${file}: a name on the big screen`);
    await page.screenshot({ path: path.join(out, `${file}.png`) });
    results.screen[file] = await page.getAttribute('.tv[data-screen]', 'data-screen');
    if (fake.box(code).writes.length) problems.push(`${file}: the screen called a host write: ${fake.box(code).writes.join(', ')}`);
  };
  {
    const { ctx, page, log, code } = await tvgo('tv-open', 'open');
    await tvShot(page, log, 'queue-tv-s1-open', code);
    const nx = await page.$$eval('.nextn li', (x) => x.map((e) => e.textContent));
    if (nx.join(',') !== '25,26,27') problems.push(`tv s1: next shows ${nx.join(',')}`);
    fake.push(code, (b) => b.callNext());
    await page.waitForFunction(() => document.querySelector('.nowbox b')?.textContent === '25', null, { timeout: 8000 }).catch(() => problems.push('tv s1: no swap to 25 after a call'));
    const said = await page.evaluate(() => document.querySelector('[aria-live]')?.textContent ?? '');
    if (!/Now serving 25/.test(said)) problems.push(`tv s1: live region says "${said}"`);
    await tvShot(page, log, 'queue-tv-s1-called', code);
    await ctx.close();
  }
  for (const [scene, screen, file] of [['tv-paused', 'paused', 'queue-tv-s2-paused'], ['tv-closed', 'closed', 'queue-tv-closed']]) {
    const { ctx, page, log, code } = await tvgo(scene, screen);
    await tvShot(page, log, file, code);
    if (scene === 'tv-closed' && (await page.$('.qqr'))) problems.push('tv closed: the QR still shows');
    await ctx.close();
  }
  {
    const { ctx, page, log, code } = await tvgo('tv-open', 'open', { reduced: true });
    fake.push(code, (b) => b.callNext());
    await page.waitForTimeout(600);
    const anim = await page.evaluate(() => [...document.querySelectorAll('.qtv *')].map((e) => getComputedStyle(e).animationName).filter((a) => a !== 'none'));
    if (anim.length) problems.push(`tv reduced motion: animates (${anim.join(' ')})`);
    await tvShot(page, log, 'queue-tv-reduced', code);
    await ctx.close();
  }
}

await browser.close();
await fake.close();
results.problems = problems;
fs.writeFileSync(path.join(out, 'queue-results.json'), JSON.stringify(results, null, 2));
console.log(JSON.stringify(results.states, null, 1), JSON.stringify(results.screen, null, 1));
if (problems.length) {
  console.error(`\n${problems.length} problem(s):\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log(`\nqueue guest page and big screen clean under the pocket CSP; shots in ${out}`);
