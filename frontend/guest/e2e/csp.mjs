// pa5 guest page in a real Chrome under exactly pa1's pocket CSP (guest/fake/views.mjs CSP), from the fake guest port.
// Fails on: any securitypolicyviolation or CSP console error, a page error, any request that is not same-origin,
// a missing or different CSP header, sideways scrolling at 390 px, or a visible tap target under 44 px.
// Also walks Secret Santa's G1-G11 (pa10a) from the fake santa box, and gather's G1-G11 (pa10b) from the fake gather box, and the front desk's G1-G12 (pa13) from the fake desk box. Writes a 390 px shot of every state and measures first paint on throttled networks (reported, not gated).
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/csp.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';
import { CSP, order } from '../fake/views.mjs';
import { santaScenarios } from '../fake/santa.mjs';
import { deskScenarios } from '../fake/desk.mjs';
import { gatherScenarios } from '../fake/gather.mjs';

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

// pa10b gather (Sign-up, RSVP and Vote), G1-G11 (pa10b/guest-brief.md), from the fake gather box (fake/gather.mjs) in
// Lin's gather.rs names-so-far. Same checks as above; shots named gather-g<N>-<what>. The gather chunk loads lazily from
// the kit in the public facts, so the walk fails if the quiz or Santa screens show instead.
{
  const gaShot = async (page, log, file) => {
    await page.evaluate(() => document.querySelector('.toast')?.remove());
    // Counted before the shot: Playwright's screenshot hides the caret by writing an empty style on inputs.
    const style = await page.evaluate(() => document.querySelectorAll('[style]').length);
    if (style) problems.push(`${file}: ${style} style attributes`);
    await shot(page, file);
    await check(file, page, log);
    results.states[file] = { screen: await page.getAttribute('[data-screen]', 'data-screen') };
  };
  const holo = (page) => page.evaluate(() => document.querySelectorAll('.kw-sticker-holo').length);
  const go = async (code, screen, { base = origin, reduced = false } = {}) => {
    const p = await newPage();
    if (reduced) await p.page.emulateMedia({ reducedMotion: 'reduce' });
    if (typeof gatherScenarios[code]?.state === 'object') {
      await p.ctx.addCookies([{ name: `kp_${code}`, value: 'fake', domain: '127.0.0.1', path: `/api/pocket/${code}`, httpOnly: true, sameSite: 'Strict' }]);
    }
    const res = await p.page.goto(`${base}/p/${code}`);
    if (res.headers()['content-security-policy'] !== CSP) problems.push(`${code}: wrong CSP header`);
    await p.page.waitForSelector(`main.ga[data-screen="${screen}"]`, { timeout: 20000 });
    return p;
  };

  // G1 RSVP, filled (Yes, +1, vegetarian), then G2 You're in
  {
    const { ctx, page, log } = await go('gather-rsvp', 'join');
    await gaShot(page, log, 'gather-g1-rsvp');
    await page.fill('#g-name', 'Ana');
    await page.click('label.opt:has(input[value="yes"])');
    await page.click('[data-k="plus-more"]');
    await page.click('.tchips label:has([data-k="food-vegetarian"])');
    await page.fill('#g-pin', '1234');
    await gaShot(page, log, 'gather-g1-rsvp-filled');
    await page.click('button[data-send]');
    await page.waitForSelector('main.ga[data-screen="in"]', { timeout: 10000 });
    await gaShot(page, log, 'gather-g2-in');
    if (await holo(page)) problems.push('gather-g2: holo on You\'re in');
    await ctx.close();
  }
  // G2 with the host's paid tick, and with counts only
  for (const [code, file] of [['gather-rsvp-in', 'gather-g2-in-paid'], ['gather-rsvp-counts', 'gather-g2-in-counts']]) {
    const { ctx, page, log } = await go(code, 'in');
    await gaShot(page, log, file);
    await ctx.close();
  }
  // G3 sign-up pick (the full slot picked: Join the waiting list)
  {
    const { ctx, page, log } = await go('gather-signup', 'join');
    await gaShot(page, log, 'gather-g3-signup');
    await page.click('label.gslot:has(input[value="2"])');
    const b = await page.textContent('button[data-send]');
    if (b !== 'Join the waiting list') problems.push(`gather-g3: full slot button says "${b}"`);
    await gaShot(page, log, 'gather-g3-signup-full');
    await ctx.close();
  }
  // Sign-up in, G4 waiting, G5 moved up (ink New, never holo)
  for (const [code, screen, file] of [['gather-signup-in', 'in', 'gather-signup-in'], ['gather-waiting', 'waiting', 'gather-g4-waiting'], ['gather-moved', 'moved', 'gather-g5-moved']]) {
    const { ctx, page, log } = await go(code, screen);
    await gaShot(page, log, file);
    if (await holo(page)) problems.push(`${file}: holo`);
    if (code === 'gather-moved' && !(await page.$('[data-sticker="ink"]'))) problems.push('gather-g5: no ink New');
    await ctx.close();
  }
  // G6 one pick
  {
    const { ctx, page, log } = await go('gather-vote', 'join');
    await page.click('label.ans:has(input[value="0"])');
    await gaShot(page, log, 'gather-g6-one-pick');
    await ctx.close();
  }
  // G7 ranked: three ranked, one left out; moving keeps focus on the moved row
  {
    const { ctx, page, log } = await go('gather-ranked', 'join');
    await gaShot(page, log, 'gather-g7-ranked-empty');
    for (const o of [0, 1, 2]) await page.click(`[data-k="add-${o}"]`);
    await page.focus('[data-k="up-1"]');
    await page.keyboard.press('Enter');
    const f = await page.evaluate(() => document.activeElement?.getAttribute('data-k'));
    if (f !== 'up-1') problems.push(`gather-g7: focus went to ${f} after a move`);
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.waitForTimeout(300);
    await gaShot(page, log, 'gather-g7-ranked');
    await ctx.close();
  }
  // G8 voted with the live count, G9 result (the one holo) and tie, G11 closed
  for (const [code, screen, file] of [['gather-voted', 'voted', 'gather-g8-voted'], ['gather-result', 'result', 'gather-g9-result'], ['gather-tie', 'result', 'gather-g9-tie'], ['gather-closed', 'closed', 'gather-g11-closed']]) {
    const { ctx, page, log } = await go(code, screen);
    await page.waitForTimeout(300);
    await gaShot(page, log, file);
    const n = await holo(page);
    if (n !== (code === 'gather-result' ? 1 : 0)) problems.push(`${file}: ${n} holo stickers`);
    await ctx.close();
  }
  // G10 rejoin with a wrong PIN
  {
    const { ctx, page, log } = await go('gather-wrongpin', 'join');
    await page.click('button[data-to-rejoin]');
    await page.waitForSelector('main.ga[data-screen="rejoin"]');
    await gaShot(page, log, 'gather-g10-rejoin');
    await page.fill('#g-bname', 'Mia');
    await page.fill('#g-bpin', '0000');
    await page.click('button[data-rejoin]');
    await page.waitForSelector('[data-problem]');
    await gaShot(page, log, 'gather-g10-wrong-pin');
    await ctx.close();
  }
  // Host asleep (served with the boot facts, as pa1 does)
  {
    const { ctx, page, log } = await go('gather-away', 'away', { base: bootOrigin });
    await gaShot(page, log, 'gather-away');
    await ctx.close();
  }
  // Reduced motion: no pick settle, no bar transition, no holo drift
  {
    const { ctx, page, log } = await go('gather-result', 'result', { reduced: true });
    const r = await page.evaluate(() => ({ bar: getComputedStyle(document.querySelector('.vb .tr i')).transitionDuration, holo: getComputedStyle(document.querySelector('.kw-sticker-holo'), '::before').animationName }));
    if (r.bar !== '0s') problems.push(`gather reduced motion: bar transition ${r.bar}`);
    if (r.holo !== 'none') problems.push(`gather reduced motion: holo drifts (${r.holo})`);
    await check('gather-reduced-motion', page, log);
    await ctx.close();
  }
}

// pa5b: the big screen at 1920x1080 (and letterboxed at 4:3), same CSP checks, from the fake screen box
// (fake/screen.mjs). Each load opens /p/screen-<scene>#screen=<token>: the token must be gone from the address bar
// before the page's first request, the screen must never call a host write, and no text may render under 32 px.
{
  const { scenes } = await import('../fake/screen.mjs');
  const screenPage = async ({ width = 1920, height = 1080, reduced = false } = {}) => {
    const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1, locale: 'en-US', reducedMotion: reduced ? 'reduce' : 'no-preference' });
    await ctx.addInitScript(() => {
      window.__violations = [];
      document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(`${e.violatedDirective} ${e.blockedURI || ''} ${e.sourceFile || ''}:${e.lineNumber || ''}`.trim()));
      // What the address bar held at each request the page made (fetch and WebSocket).
      window.__hashAt = [];
      const f = window.fetch;
      window.fetch = (...a) => {
        window.__hashAt.push(`${location.hash} ${String(a[0])}`);
        return f(...a);
      };
      const W = window.WebSocket;
      window.WebSocket = class extends W {
        constructor(u, p) {
          window.__hashAt.push(`${location.hash} ${String(u)}`);
          super(u, p);
        }
      };
    });
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
  };
  const screenCheck = async (name, page, log, { width = 1920, height = 1080 } = {}) => {
    for (const x of await page.evaluate(() => window.__violations)) problems.push(`${name}: CSP violation: ${x}`);
    for (const x of log) problems.push(`${name}: ${x}`);
    const hashes = await page.evaluate(() => ({ now: location.href, at: window.__hashAt }));
    if (hashes.now.includes('#')) problems.push(`${name}: the address bar still has a fragment (${hashes.now})`);
    for (const x of hashes.at) if (x.startsWith('#')) problems.push(`${name}: a request went out with the token still in the address bar: ${x}`);
    const m = await page.evaluate(() => {
      const tv = document.querySelector('.tv');
      const s = Number(getComputedStyle(tv).getPropertyValue('--s')) || 1;
      const small = [];
      for (const el of tv.querySelectorAll('*')) {
        if (el.closest('.vh, svg')) continue;
        const own = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
        if (!own) continue;
        const r = el.getBoundingClientRect();
        if (!r.width || !r.height) continue;
        // The sticker's own 12 px type is drawn at scale(3.2) (38.4 px at 1920).
        const k = el.closest('.bigsticker') ? 3.2 : 1;
        const px = parseFloat(getComputedStyle(el).fontSize) * k;
        if (px < 32) small.push(`${el.tagName.toLowerCase()}.${el.className} "${el.textContent.trim().slice(0, 24)}" ${px}px`);
      }
      const box = tv.getBoundingClientRect();
      const unfilled = [...document.querySelectorAll('svg rect')].filter((r) => {
        const f = r.getAttribute('fill');
        return f !== 'currentColor' && !/^#[0-9a-f]{6}$/i.test(f ?? '');
      }).length;
      return { s, small, box: [box.left, box.top, box.width, box.height].map(Math.round), sw: document.documentElement.scrollWidth, sh: document.documentElement.scrollHeight, unfilled };
    });
    for (const x of m.small) problems.push(`${name}: text under 32 px at 1920: ${x}`);
    if (m.sw > width || m.sh > height) problems.push(`${name}: the page scrolls (${m.sw}x${m.sh})`);
    if (m.box[2] > width + 1 || m.box[3] > height + 1) problems.push(`${name}: the stage overflows the window (${m.box.join(' ')})`);
    if (m.unfilled) problems.push(`${name}: ${m.unfilled} svg rect(s) with no real fill`);
    return m;
  };
  const shots = [['lobby', 'lobby'], ['question', 'question'], ['locked', 'question'], ['reveal', 'reveal'], ['leaderboard', 'leaderboard'], ['winners', 'winners'], ['paused', 'paused'], ['away', 'away'], ['ended', 'winners'], ['expired', 'signed-out'], ['solo', 'leaderboard']];
  results.screen = {};
  for (const [i, [scene, screen]] of shots.entries()) {
    const code = `screen-${scene}`;
    const { ctx, page, log } = await screenPage();
    // The solo board's creatures come from the lobby roster, so that screen first sees its lobby, as a real TV does.
    const solo = scene === 'solo';
    if (solo) fake.push(code, { ...scenes.lobby, teams: false, lobby: scenes.solo.leaderboard.map((r) => ({ name: r.name, team: null })), players: scenes.solo.leaderboard.length });
    await page.goto(`${origin}/p/${code}#screen=tok-${scene}`);
    await ready(page, solo ? 'lobby' : screen);
    if (solo) {
      fake.push(code, scenes.solo);
      await ready(page, screen);
    }
    await page.waitForTimeout(400);
    const file = `screen-${String(i + 1).padStart(2, '0')}-${scene}`;
    const mm = await screenCheck(file, page, log);
    await page.screenshot({ path: path.join(out, `${file}.png`) });
    results.screen[scene] = { screen, scale: mm.s, stage: mm.box };
    const b = fake.box(code);
    if (b.writes?.length) problems.push(`${file}: the screen called a host write: ${b.writes.join(', ')}`);
    if (scene !== 'away' && scene !== 'expired' && !(b.signins?.length === 1 && b.signins[0].key === `tok-${scene}`)) problems.push(`${file}: signed in ${JSON.stringify(b.signins)}`);
    await ctx.close();
  }
  // A new player drops in on the live lobby (ws push); a reload of the tab stays a screen (cookie, no token).
  {
    const { ctx, page, log } = await screenPage();
    await page.goto(`${origin}/p/screen-lobby#screen=tok-again`);
    await ready(page, 'lobby');
    await page.waitForTimeout(300);
    fake.push('screen-lobby', { ...scenes.lobby, lobby: [...scenes.lobby.lobby, { name: 'Eli', team: 'Brain Freeze' }], players: 15 });
    await page.waitForSelector('.player.drop-in', { timeout: 5000 }).catch(() => problems.push('screen-lobby: no drop-in after a ws push'));
    await page.reload();
    await ready(page, 'lobby');
    await screenCheck('screen-lobby-reload', page, log);
    if (!(await page.textContent('.tvcount')).includes('15 players')) problems.push('screen-lobby-reload: not the live lobby');
    await ctx.close();
  }
  // Letterbox at 4:3 and on an ultra-wide.
  for (const [w, hgt, scene] of [[1024, 768, 'lobby'], [2560, 1080, 'leaderboard']]) {
    const { ctx, page, log } = await screenPage({ width: w, height: hgt });
    await page.goto(`${origin}/p/screen-${scene}#screen=tok-lb`);
    await ready(page, scene);
    await page.waitForTimeout(300);
    await screenCheck(`screen-letterbox-${w}x${hgt}`, page, log, { width: w, height: hgt });
    await page.screenshot({ path: path.join(out, `screen-letterbox-${w}x${hgt}-${scene}.png`) });
    await ctx.close();
  }
  // pa5b round 2 (Rigel D1): the validator allows 2 to 6 answers, a 300-character question, 80-character answers and a
  // 280-character line. No answer may sit under the answered row or the Quizmaster, and every block stays on the stage.
  {
    const words = (n) => {
      const w = 'the quick brown fox jumps over a lazy dog while seven wizards quietly box'.split(' ');
      let t = '';
      for (let i = 0; t.length < n; i++) t += (t ? ' ' : '') + w[i % w.length];
      return t.slice(0, n).trim();
    };
    const q = scenes.question.question;
    const cases = [
      ['5 short answers', 'question', { ...scenes.question, question: { ...q, options: ['One', 'Two', 'Three', 'Four', 'Five'] } }],
      ['6 short answers', 'question', { ...scenes.question, question: { ...q, options: ['One', 'Two', 'Three', 'Four', 'Five', 'Six'] } }],
      ['140-character question, 4x30', 'question', { ...scenes.question, question: { ...q, text: words(140), options: [30, 30, 30, 30].map(words) } }],
      ['locked, 6 answers', 'locked', { ...scenes.locked, question: { ...q, options: ['One', 'Two', 'Three', 'Four', 'Five', 'Six'] } }],
      ['limits: 300 + 6x80', 'question', { ...scenes.question, question: { ...q, text: words(300), options: Array.from({ length: 6 }, () => words(80)) } }],
      ['reveal 6 short answers', 'reveal', { ...scenes.reveal, question: { ...q, options: ['One', 'Two', 'Three', 'Four', 'Five', 'Six'] }, reveal: { ...scenes.reveal.reveal, picks: [1, 14, 5, 2, 0, 1] } }],
      ['reveal limits: 300 + 4x80 + 280 line', 'reveal', { ...scenes.reveal, question: { ...q, text: words(300), options: [80, 80, 80, 80].map(words) }, reveal: { ...scenes.reveal.reveal, source_quote: words(300), line: words(280) } }],
      // pa5b round 3 (Rigel D2 and D3): the source quote counts, and 5 or 6 answers of 28 to 30 characters can wrap.
      ['D3 question 120 + 5x30', 'question', { ...scenes.question, question: { ...q, text: words(120), options: Array.from({ length: 5 }, () => words(30)) } }],
      ['D3 locked 120 + 6x28', 'locked', { ...scenes.locked, question: { ...q, text: words(120), options: Array.from({ length: 6 }, () => words(28)) } }],
      ['D2 reveal 120 + 5x30 + quote 100', 'reveal', { ...scenes.reveal, question: { ...q, text: words(120), options: Array.from({ length: 5 }, () => words(30)) }, reveal: { ...scenes.reveal.reveal, picks: [1, 14, 5, 2, 0], source_quote: words(100) } }],
      ['D2 reveal 120 + 4x50 + quote 300', 'reveal', { ...scenes.reveal, question: { ...q, text: words(120), options: [50, 50, 50, 50].map(words) }, reveal: { ...scenes.reveal.reveal, source_quote: words(300) } }],
      ['every field at the limits: reveal 300 + 6x80 + quote 300 + line 280', 'reveal', { ...scenes.reveal, question: { ...q, text: words(300), options: Array.from({ length: 6 }, () => words(80)) }, reveal: { ...scenes.reveal.reveal, picks: [0, 1, 2, 3, 4, 5], source_quote: words(300), line: words(280) } }],
    ];
    for (const [name, scene, view] of cases) {
      const code = `screen-${scene}`;
      fake.push(code, scenes[scene]);
      const { ctx, page, log } = await screenPage();
      await page.goto(`${origin}/p/${code}#screen=tok-limits`);
      await ready(page, scene === 'locked' ? 'question' : scene);
      fake.push(code, view);
      await page.waitForTimeout(400);
      await screenCheck(`screen-limits ${name}`, page, log);
      const bad = await page.evaluate(() => {
        const out = [];
        const tv = document.querySelector('.tv').getBoundingClientRect();
        const ans = [...document.querySelectorAll('.tv .ans, .tv .tvsolo')].map((e) => e.getBoundingClientRect());
        for (const sel of ['.tvanswered', '.tvq > .tvhost', '.tvsrc']) {
          const b = document.querySelector(sel)?.getBoundingClientRect();
          if (b && ans.some((a) => a.bottom > b.top + 1 && a.top < b.bottom - 1)) out.push(`an answer overlaps ${sel}`);
        }
        for (const e of document.querySelectorAll('.tv .ans, .tv .tvanswered, .tv .tvhost, .tv .tvsrc, .tv .qbig, .tv .tvclock')) {
          const r = e.getBoundingClientRect();
          if (r.bottom > tv.bottom + 1 || r.right > tv.right + 1 || r.top < tv.top - 1) out.push(`${e.className} leaves the stage`);
        }
        const col = document.querySelector('.tvq');
        if (col && col.scrollHeight > col.clientHeight + 1) out.push(`the question column overflows by ${col.scrollHeight - col.clientHeight}px (${col.className})`);
        const clock = document.querySelector('.tvclock')?.getBoundingClientRect();
        const head = document.querySelector('.tvtop')?.getBoundingClientRect();
        const q = document.querySelector('.tvq')?.getBoundingClientRect();
        if (clock && q && clock.bottom > q.top) out.push('the clock runs into the question');
        if (clock && head && head.right > clock.left) out.push('the header runs under the clock');
        return out;
      });
      for (const x of bad) problems.push(`screen-limits ${name}: ${x}`);
      await page.screenshot({ path: path.join(out, `screen-limits-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`) });
      await ctx.close();
    }
  }
  // Reduced motion: the winners' stars hold still, the timer bar and new players don't move.
  {
    const { ctx, page, log } = await screenPage({ reduced: true });
    await page.goto(`${origin}/p/screen-winners#screen=tok-rm`);
    await ready(page, 'winners');
    const anim = await page.evaluate(() => [...document.querySelectorAll('.tvstar')].map((s) => getComputedStyle(s).animationName));
    if (anim.some((a) => a !== 'none')) problems.push(`screen reduced motion: stars animate (${anim.join(' ')})`);
    await screenCheck('screen-reduced-motion', page, log);
    await ctx.close();
  }
}

// First paint on throttled networks. DevTools "Slow 4G" (562.5 ms, 180 KB/s down) and Lighthouse's slow 4G
// (150 ms, 1.6 Mbps). Reported in pa5/impl.md, not gated: loopback plus CDP throttling is a model, not a phone.
const presets = {
  'devtools-slow-4g': { latency: 562.5, downloadThroughput: 180000, uploadThroughput: 84375 },
  'lighthouse-slow-4g': { latency: 150, downloadThroughput: 204800, uploadThroughput: 84375 },
};
// pa13 front desk guest chat, G1-G12 (pa13/guest-brief.md), from the fake desk box (fake/desk.mjs) in Lin's impl.md
// shapes. Same checks as the quiz screens; shots named desk-g<N>-<what>. The kit comes from the public facts, so the
// desk chunk loads lazily, and the walk fails if the quiz screens show instead.
{
  const deskShot = async (page, log, file) => {
    await shot(page, file);
    await check(file, page, log);
    results.states[file] = { screen: await page.getAttribute('[data-screen]', 'data-screen') };
  };
  const go = async (code, screen, { reduced = false } = {}) => {
    const p = await newPage();
    if (reduced) await p.page.emulateMedia({ reducedMotion: 'reduce' });
    if (typeof deskScenarios[code]?.state === 'object') {
      await p.ctx.addCookies([{ name: `kp_${code}`, value: 'fake', domain: '127.0.0.1', path: `/api/pocket/${code}`, httpOnly: true, sameSite: 'Strict' }]);
    }
    const res = await p.page.goto(`${origin}/p/${code}`);
    if (res.headers()['content-security-policy'] !== CSP) problems.push(`${code}: wrong CSP header`);
    await p.page.waitForSelector(`main.dk[data-screen="${screen}"]`, { timeout: 20000 });
    return p;
  };
  const want = async (page, sel, what) => {
    try {
      await page.waitForSelector(sel, { timeout: 8000 });
    } catch {
      problems.push(`desk: ${what} (${sel}) not shown`);
    }
  };
  const noText = async (page, re, what) => {
    const t = await page.evaluate(() => document.body.innerText);
    if (re.test(t)) problems.push(`desk: ${what}`);
  };

  // G1 join, then G2 first time with starters; a starter sends and Bell types (G3)
  {
    const { ctx, page, log } = await go('desk-join', 'join');
    await deskShot(page, log, 'desk-g1-join');
    await page.fill('#d-name', 'Ana');
    await page.fill('#d-pin', '1234');
    await page.click('button[data-join]');
    await page.waitForSelector('main.dk[data-screen="chat"]', { timeout: 10000 });
    await want(page, '[data-starter]', 'G2 starter chips');
    await deskShot(page, log, 'desk-g2-first');
    await page.click('[data-starter]');
    await want(page, '[data-typing]', 'G3 typing after a starter');
    const mine = await page.locator('.gm.me').count();
    if (mine !== 1) problems.push(`desk G3: ${mine} own bubbles after one send (want 1: the outbox bubble swaps for the kept one)`);
    // The fake's reply arrives over the socket: typing stops, Bell answers.
    fake.push('desk-join', (v) => {
      v.bot_typing = false;
      v.messages.push({ id: 'm900', from: 'bot', text: 'The Wi-Fi is "SeaView-5G", password piran2026.', at: new Date().toISOString() });
    });
    await want(page, '.gm[data-id="m900"]', 'G3 the pushed bot reply');
    await ctx.close();
  }
  // G2 first time from a saved seat, G3 typing, G4 hand-off, G5 host, G7 daily cap, G10 host away
  for (const [code, file, sel] of [
    ['desk-first', 'desk-g2-first-seat', '[data-starter]'],
    ['desk-chat', 'desk-g3-typing', '[data-typing]'],
    ['desk-handoff', 'desk-g4-handoff', '[data-note="handoff"]'],
    ['desk-host', 'desk-g5-host', '.hosttag'],
    ['desk-cap', 'desk-g7-cap', '[data-note="daily_cap"]'],
    ['desk-away', 'desk-g10-away', '[data-note="host_away"]'],
  ]) {
    const { ctx, page, log } = await go(code, 'chat');
    await want(page, sel, file);
    if (code === 'desk-handoff' || code === 'desk-cap' || code === 'desk-away') await want(page, '[data-waiting]', `${file} waiting pill`);
    // Never shown to a guest: the hand-off reason, another guest, answer_bot's id.
    await noText(page, /handoff_reason|answer_bot|undefined|null/, `${file}: internal words on screen`);
    await deskShot(page, log, file);
    await ctx.close();
  }
  // G6 slow down: the send is refused with retry_at; the text stays in the composer
  {
    const { ctx, page, log } = await go('desk-slow', 'chat');
    await page.fill('#d-say', 'Is there a hair dryer?');
    await page.click('button[data-send]');
    await want(page, '[data-slow]', 'G6 slow note');
    const kept = await page.inputValue('#d-say');
    if (kept !== 'Is there a hair dryer?') problems.push(`desk G6: composer lost the text on 429 ("${kept}")`);
    if (!(await page.isDisabled('button[data-send]'))) problems.push('desk G6: send stays on while slowed down');
    await deskShot(page, log, 'desk-g6-slow');
    await ctx.close();
  }
  // G8 too long: over the limit, send turns off and the field says so
  {
    const { ctx, page, log } = await go('desk-long', 'chat');
    await page.fill('#d-say', 'The sea view is lovely. '.repeat(44));
    await page.waitForTimeout(100);
    if (!(await page.isDisabled('button[data-send]'))) problems.push('desk G8: send stays on over 1,000 characters');
    if ((await page.getAttribute('#d-say', 'aria-invalid')) !== 'true') problems.push('desk G8: field not marked invalid');
    await deskShot(page, log, 'desk-g8-long');
    await ctx.close();
  }
  // G9 offline: the action fails, the bubble says Not sent, the bar offers Retry
  {
    const { ctx, page, log } = await go('desk-offline', 'chat');
    await page.fill('#d-say', 'Where is the nearest bakery?');
    await page.click('button[data-send]');
    await want(page, '[data-try-one]', 'G9 Not sent bubble');
    await want(page, '[data-offline]', 'G9 offline bar');
    await deskShot(page, log, 'desk-g9-offline');
    await ctx.close();
  }
  // G1 full: guest_max says the guide is full, no seat
  {
    const { ctx, page, log } = await go('desk-full', 'join');
    await page.fill('#d-name', 'Ana');
    await page.fill('#d-pin', '1234');
    await page.click('button[data-join]');
    await want(page, 'text=This guide is full right now.', 'G1 guest_max copy');
    await deskShot(page, log, 'desk-g1-full');
    await ctx.close();
  }
  // G11 rejoin: a wrong name first, then the right one opens the chat
  {
    const { ctx, page, log } = await go('desk-rejoin', 'join');
    await page.click('[data-to-rejoin]');
    await page.waitForSelector('main.dk[data-screen="rejoin"]', { timeout: 5000 });
    await deskShot(page, log, 'desk-g11-rejoin');
    await page.fill('#d-bname', 'Anna');
    await page.fill('#d-bpin', '1234');
    await page.click('button[data-rejoin]');
    await want(page, 'text=No chat with that name here.', 'G11 unknown name');
    await page.fill('#d-bname', 'Ana');
    await page.fill('#d-bpin', '1234');
    await page.click('button[data-rejoin]');
    await page.waitForSelector('main.dk[data-screen="chat"]', { timeout: 10000 });
    await ctx.close();
  }
  // G12 ended
  {
    const { ctx, page, log } = await go('desk-ended', 'ended');
    await want(page, '[data-ended]', 'G12 ended card');
    await deskShot(page, log, 'desk-g12-ended');
    await ctx.close();
  }
  // Reduced motion: no bubble entrance, no typing dots moving
  {
    const { ctx, page, log } = await go('desk-chat', 'chat', { reduced: true });
    const anims = await page.evaluate(() => [...document.querySelectorAll('main.dk *')].map((e) => getComputedStyle(e).animationName).filter((a) => a && a !== 'none'));
    if (anims.length) problems.push(`desk reduced motion: ${anims.length} element(s) animate (${[...new Set(anims)].join(', ')})`);
    await check('desk-reduced-motion', page, log);
    await ctx.close();
  }
}
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
