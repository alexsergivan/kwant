// pa5 guest page in a real Chrome under exactly pa1's pocket CSP (guest/fake/views.mjs CSP), from the fake guest port.
// Fails on: any securitypolicyviolation or CSP console error, a page error, any request that is not same-origin,
// a missing or different CSP header, sideways scrolling at 390 px, or a visible tap target under 44 px.
// Also walks Secret Santa's G1-G11 (pa10a) from the fake santa box, and the Wall's G1-G8 and TV1-TV4 (pa14) from the fake wall box. Writes a 390 px shot of every state and measures first paint on throttled networks (reported, not gated).
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
    // blob: of this origin is the Wall's own preview of a photo the guest picked (URL.createObjectURL), never a fetch out.
    if (!u.startsWith(`http://127.0.0.1:`) && !u.startsWith('ws://127.0.0.1:') && !u.startsWith('data:') && !u.startsWith('blob:http://127.0.0.1:')) log.push(`third-party request: ${u}`);
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

// pa14 the Wall's guest page, G1-G8 (pa14/guest-brief.md), from the fake wall box (fake/wall.mjs) in Juno's wall.rs
// shapes. Same checks as the quiz screens; shots named wall-g<N>-<what>. Uploads are real files through a real XHR.
{
  // Real small JPEGs drawn in Chrome (no image tools on the runner), one colour each, so the previews show.
  const jpegBytes = [];
  {
    const p = await newPage();
    for (const [i, c] of ['#c07a5a', '#5a7ac0', '#7ac05a', '#c05a9a', '#c0b05a'].entries()) {
      const b64 = await p.page.evaluate(([c, i]) => {
        const k = document.createElement('canvas');
        k.width = 120;
        k.height = 160;
        const g = k.getContext('2d');
        g.fillStyle = c;
        g.fillRect(0, 0, 120, 160);
        g.fillStyle = '#f4f1ff';
        g.beginPath();
        g.arc(40 + i * 10, 50, 22, 0, 7);
        g.fill();
        return k.toDataURL('image/jpeg', 0.8).split(',')[1];
      }, [c, i]);
      jpegBytes.push(Buffer.from(b64, 'base64'));
    }
    await p.ctx.close();
  }
  let jn = 0;
  const jpeg = (name) => ({ name, mimeType: 'image/jpeg', buffer: jpegBytes[jn++ % jpegBytes.length] });
  const heic = (name) => ({ name, mimeType: 'image/heic', buffer: Buffer.from('fake heic'.padEnd(400, '.')) });
  const wallShot = async (page, log, file) => {
    await page.evaluate(() => document.querySelector('.toast')?.remove());
    // The wall's photos load lazily: scroll through once so a full-page shot has them all, then back to where it was.
    const y = await page.evaluate(() => window.scrollY);
    await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
    await page.waitForTimeout(400);
    await page.evaluate((y) => window.scrollTo(0, y), y);
    await page.waitForFunction(() => [...document.querySelectorAll('[data-wall] img, [data-mine] img')].every((i) => i.complete), null, { timeout: 5000 }).catch(() => {});
    await shot(page, file);
    await check(file, page, log);
    results.states[file] = { screen: await page.getAttribute('[data-screen]', 'data-screen') };
    // The page root must stay a block: a stray grid (Santa's .wl once matched it) stretches the bar on short screens.
    const barH = await page.evaluate(() => document.querySelector('.g > .bar')?.getBoundingClientRect().height ?? 0);
    if (!barH || barH > 64) problems.push(`${file}: the header is ${Math.round(barH)} px tall (at most 64)`);
    if (!file.startsWith('wall-g6') && (await page.$('.kw-sticker-holo'))) problems.push(`${file}: holo off the Picked screen`);
    // CSSOM setProperty of a custom property (the upload bar's --p) is allowed under the CSP, and Chrome leaves an empty
    // style="" on inputs it may autofill; anything else is not.
    const style = await page.evaluate(() => [...document.querySelectorAll('[style]')].map((e) => `${e.tagName.toLowerCase()}.${e.className} "${e.getAttribute('style')}"`).filter((x) => !/""$/.test(x) && !/"(\s*--[\w-]+:\s*[\d.]+;?)+\s*"$/.test(x)));
    for (const x of style) problems.push(`${file}: a style that is not a CSSOM custom property: ${x}`);
  };
  const wgo = async (code, screen, { base = origin } = {}) => {
    const p = await newPage();
    const res = await p.page.goto(`${base}/p/${code}`);
    if (res.headers()['content-security-policy'] !== CSP) problems.push(`${code}: wrong CSP header`);
    await p.page.waitForSelector(`main.wall[data-screen="${screen}"]`, { timeout: 25000 });
    await p.page.waitForTimeout(300);
    return p;
  };
  const files = (page) => page.locator('input[type=file]').first();

  // G1 join (keep line, limits), files picked before joining wait, then join sends them (G2) and the thanks shows.
  {
    const { ctx, page, log } = await wgo('wall-join', 'join');
    await wallShot(page, log, 'wall-g1-join');
    await files(page).setInputFiles([jpeg('a.jpg'), jpeg('b.jpg')]);
    await page.waitForSelector('text=They send once you join');
    await page.fill('#w-name', 'Sam');
    await page.click('#w-pin');
    await page.keyboard.type('1234');
    await page.focus('#w-name');
    await wallShot(page, log, 'wall-g1-join-filled');
    await page.locator('form button[type=submit]').first().click();
    await page.waitForSelector('main.wall[data-screen="wall"]', { timeout: 10000 });
    await page.waitForSelector('text=Thanks, Sam!', { timeout: 10000 });
    await wallShot(page, log, 'wall-g2-sent');
    await ctx.close();
  }
  // G2 and G4: one at a time, refused rows in place with a fix, the rest go up.
  {
    const { ctx, page, log } = await wgo('wall-refuse', 'wall');
    // The page refuses the HEIC itself, before sending, so the box's own "heic" answer would land on the next file.
    fake.push('wall-refuse', (b) => (b.s = { ...b.s, uploads: b.s.uploads.filter((u) => u.error !== 'heic') }));
    await files(page).setInputFiles([jpeg('ok-1.jpg'), heic('IMG_0042.heic'), jpeg('big.jpg'), jpeg('ok-2.jpg'), jpeg('ok-3.jpg')]);
    await page.waitForSelector('.sending', { timeout: 5000 });
    await page.waitForTimeout(400);
    await wallShot(page, log, 'wall-g2-sending');
    await page.waitForSelector('[data-refused]', { timeout: 20000 });
    await page.waitForTimeout(3500);
    await wallShot(page, log, 'wall-g4-refused');
    const txt = await page.textContent('main');
    if (/\d+(\.\d+)? ?(KB|MB) (of|sent)|\d%/.test(txt)) problems.push('wall-g4: a size or percentage shows');
    await ctx.close();
  }
  // G4 with Try again: the box answers 400 broken_photo; the row's Try again resends and it goes up.
  {
    const { ctx, page, log } = await wgo('wall-broken', 'wall');
    await files(page).setInputFiles([jpeg('ok-1.jpg')]);
    await page.waitForSelector('li[data-state="refused"] button.retry', { timeout: 20000 });
    await page.waitForTimeout(300);
    await wallShot(page, log, 'wall-g4-try-again');
    await page.click('li[data-state="refused"] button.retry');
    await page.waitForFunction(() => !document.querySelector('li[data-state="refused"]'), null, { timeout: 20000 });
    await ctx.close();
  }
  // G3 the live wall: yours first, newest first, 12 then Show more; a new approved photo waits behind the pill.
  {
    const { ctx, page, log } = await wgo('wall-live', 'wall');
    await wallShot(page, log, 'wall-g3-live');
    const n = await page.$$eval('[data-wall] .ph', (x) => x.length);
    if (n !== 12) problems.push(`wall-g3: ${n} tiles, not 12`);
    if (/\b15 photos\b/.test(await page.textContent('main'))) problems.push('wall-g3: shows a count of others');
    fake.push('wall-live', (b) => b.approve({ id: 'new1', by_name: 'Zoe', caption: 'Cake time', caption_removed: false, at: '2026-11-28T21:40:00Z', w: 1200, h: 1600, rotate: 0, best: false }));
    await page.waitForSelector('.newpill', { timeout: 8000 }).catch(() => problems.push('wall-g3: no new-photo pill after a push'));
    await page.evaluate(() => window.scrollTo(0, 0));
    await wallShot(page, log, 'wall-g3-new-pill');
    await ctx.close();
  }
  // G5 back: the rejoin form.
  {
    const { ctx, page, log } = await wgo('wall-join', 'join');
    await page.click("text=I've joined before");
    await page.waitForSelector('main.wall[data-screen="rejoin"]');
    await wallShot(page, log, 'wall-g5-rejoin');
    await ctx.close();
  }
  // G7 closed with a seat, then G6 the best: one holo Picked sticker.
  {
    const { ctx, page, log } = await wgo('wall-best', 'closed');
    await wallShot(page, log, 'wall-g7-closed-seated');
    await page.click('text=See the best');
    await page.waitForSelector('main.wall[data-screen="best"]');
    await page.waitForTimeout(300);
    await wallShot(page, log, 'wall-g6-best');
    if ((await page.$$('.kw-sticker-holo')).length !== 0) problems.push('wall-g6: a holo sticker on a host-picked best set');
    await ctx.close();
  }
  // G7 closed, never joined.
  {
    const { ctx, page, log } = await wgo('wall-closed', 'closed');
    await wallShot(page, log, 'wall-g7-closed');
    await ctx.close();
  }
  // G8 page notes: the wall is full, the guest max.
  for (const [code, file] of [['wall-full', 'wall-g8-full'], ['wall-max', 'wall-g8-max']]) {
    const { ctx, page, log } = await wgo(code, 'wall');
    await files(page).setInputFiles([jpeg('a.jpg')]);
    await page.waitForSelector('[data-note]', { timeout: 10000 });
    await page.evaluate(() => window.scrollTo(0, 0));
    await wallShot(page, log, file);
    await ctx.close();
  }
  // Empty wall, ended, away (served with the boot facts, as pa1 does).
  for (const [code, file, base] of [['wall-empty', 'wall-empty', origin], ['wall-ended', 'wall-ended', origin], ['wall-away', 'wall-away', bootOrigin]]) {
    const p = await newPage();
    await p.page.goto(`${base}/p/${code}`);
    const ok = await p.page.waitForSelector('[data-screen]:not([data-screen="loading"])', { timeout: 25000 }).catch(() => null);
    if (!ok) problems.push(`${file}: never left loading`);
    else {
      await p.page.waitForTimeout(code === 'wall-away' ? 6000 : 300);
      await wallShot(p.page, p.log, file);
    }
    await p.ctx.close();
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
  // pa14 the Wall's big screen, TV1-TV4 at 1920 (pa14/guest-brief.md), from the fake wall box in its screen view.
  {
    const tvShot = async (page, log, file, code, srv = fake) => {
      await page.waitForTimeout(400);
      await screenCheck(file, page, log);
      await page.screenshot({ path: path.join(out, `${file}.png`) });
      results.screen[file] = { screen: await page.getAttribute('.tv[data-screen]', 'data-screen') };
      const b = srv.box(code);
      if (b.writes?.length) problems.push(`${file}: the screen called a host write: ${b.writes.join(', ')}`);
    };
    const tvgo = async (scene, screen, opts = {}) => {
      const p = await screenPage(opts);
      await p.page.goto(`${opts.base ?? origin}/p/screen-wall-${scene}#screen=tok-wall-${scene}`);
      await ready(p.page, screen);
      return p;
    };
    // TV1 slides, then TV2 a new photo arrives (pill and move), from a push.
    {
      const { ctx, page, log } = await tvgo('slides', 'slides');
      await tvShot(page, log, 'wall-tv1-slides', 'screen-wall-slides');
      fake.push('screen-wall-slides', (b) => b.approve({ id: 'tvnew1', by_name: 'Zoe', caption: 'Cake time', caption_removed: false, at: '2026-11-28T21:40:00Z', w: 1200, h: 1600, rotate: 0, best: false }));
      await page.waitForSelector('.wnew', { timeout: 8000 }).catch(() => problems.push('wall-tv2: no "New from" pill after a push'));
      await page.waitForTimeout(250);
      await tvShot(page, log, 'wall-tv2-arrival', 'screen-wall-slides');
      const said = await page.evaluate(() => [...document.querySelectorAll('[aria-live]')].map((e) => e.textContent).join(' | '));
      if (!/New photo from Zoe/.test(said)) problems.push(`wall-tv2: live region says "${said}"`);
      await ctx.close();
    }
    // TV3 waiting: no photos yet, the big code.
    {
      const { ctx, page, log } = await tvgo('waiting', 'waiting');
      await tvShot(page, log, 'wall-tv3-waiting', 'screen-wall-waiting');
      await ctx.close();
    }
    // TV4 reduced motion: the still grid, one slot changes every 8 s, no animation.
    {
      const { ctx, page, log } = await tvgo('slides', 'still', { reduced: true });
      await tvShot(page, log, 'wall-tv4-still', 'screen-wall-slides');
      const anim = await page.evaluate(() => [...document.querySelectorAll('.wtv *')].map((e) => getComputedStyle(e).animationName).filter((a) => a !== 'none'));
      if (anim.length) problems.push(`wall-tv4: animates under reduced motion (${anim.join(' ')})`);
      await ctx.close();
    }
    // Ended and away, then the waiting screen letterboxed at 4:3.
    // Away is served the way pa1 serves it, with the public facts in the page, so the screen knows it is the wall's.
    for (const [scene, screen, base] of [['ended', 'ended', origin], ['away', 'away', bootOrigin]]) {
      const { ctx, page, log } = await tvgo(scene, screen, { base });
      await tvShot(page, log, `wall-tv-${scene}`, `screen-wall-${scene}`, base === bootOrigin ? boot : fake);
      if (!(await page.$(`.wtv[data-screen="${screen}"]`))) problems.push(`wall-tv-${scene}: not the wall's own ${screen} screen`);
      await ctx.close();
    }
    {
      const { ctx, page, log } = await tvgo('waiting', 'waiting', { width: 1024, height: 768 });
      await page.waitForTimeout(300);
      await screenCheck('wall-tv-letterbox-1024x768', page, log, { width: 1024, height: 768 });
      await page.screenshot({ path: path.join(out, 'wall-tv-letterbox-1024x768-waiting.png') });
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
