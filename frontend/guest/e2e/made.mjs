// pa12 P2: the ready-made blocks in a real Chrome under exactly pa1's pocket CSP, from the fake made box
// (fake/made.mjs). Every block on the phone at 390 px, the big screen at 1920x1080 and the host remote at 390 px, from
// the two section-1 examples (book shelf + vote, guess-the-opening-line), plus a walk: join, add a book, vote, take it
// back, send a guess, and the host revealing. Fails on: a CSP violation or page error, a request that is not
// same-origin, sideways scrolling at 390, a visible tap target under 44 px, markup from guest text, a control on the
// big screen.
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/made.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';

const pw = await import(process.env.PW_CORE || 'playwright-core');
const out = path.resolve(process.argv[2] || 'guest-shots');
fs.mkdirSync(out, { recursive: true });
const fake = await startFake({ port: 0, actionDelayMs: 150, boot: true });
const origin = `http://127.0.0.1:${fake.port}`;
const browser = await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox'] });
const problems = [];
const results = {};
let n = 0;

async function open(code, { width = 390, height = 844, hash = '', reduced = false } = {}) {
  const mobile = width < 800;
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: mobile ? 2 : 1, isMobile: mobile, hasTouch: mobile, locale: 'en-US', timezoneId: 'Europe/Ljubljana', reducedMotion: reduced ? 'reduce' : 'no-preference' });
  await ctx.addInitScript(() => {
    window.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(`${e.violatedDirective} ${e.blockedURI || ''}`.trim()));
  });
  const page = await ctx.newPage();
  const log = [];
  page.on('pageerror', (e) => log.push(`pageerror: ${e.message}`));
  page.on('console', (m) => /Content.Security.Policy|Refused to/i.test(m.text()) && log.push(`console: ${m.text()}`));
  page.on('request', (r) => !r.url().startsWith(origin) && !r.url().startsWith('data:') && log.push(`off-origin: ${r.url()}`));
  const url = `${origin}/p/${code}--${++n}${hash}`;
  await page.goto(url);
  return { ctx, page, log, url };
}

async function check(name, { page, log }, { width }) {
  const facts = await page.evaluate(() => {
    const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const small = [...document.querySelectorAll('button, input:not([type=radio]):not([type=checkbox]), label.mopt')]
      .filter(vis).map((e) => [e.textContent.trim().slice(0, 30) || e.id, Math.round(e.getBoundingClientRect().height)]).filter(([, hh]) => hh < 44);
    return {
      sideways: document.documentElement.scrollWidth > innerWidth,
      small,
      violations: window.__violations,
      injected: document.querySelectorAll('img, script:not([type="application/json"]):not([src]), iframe').length,
      controls: document.querySelectorAll('main[data-surface=screen] button, main[data-surface=screen] input').length,
      blocks: [...document.querySelectorAll('[data-block]')].map((e) => e.dataset.block),
      screen: document.querySelector('main')?.dataset.screen,
      // The big screen has no scrolling: a card or line cut off at the bottom would just be hidden.
      clipped: document.querySelector('main[data-surface=screen]') ? [...document.querySelectorAll('[data-block], .body.mid > *')].filter((e) => e.getBoundingClientRect().bottom > innerHeight + 1).map((e) => e.dataset.block || e.tagName) : [],
    };
  });
  const bad = [...log, ...facts.violations.map((v) => `csp: ${v}`)];
  if (width <= 400 && facts.sideways) bad.push('scrolls sideways');
  if (width <= 400 && facts.small.length) bad.push(`small targets: ${JSON.stringify(facts.small)}`);
  if (facts.injected) bad.push(`${facts.injected} element(s) from data`);
  if (facts.controls) bad.push('a control on the big screen');
  if (facts.clipped.length) bad.push(`cut off at the bottom: ${facts.clipped}`);
  for (const b of bad) problems.push(`${name}: ${b}`);
  results[name] = { blocks: facts.blocks, screen: facts.screen, problems: bad };
}

async function shot(name, code, opts = {}, ready = '[data-screen=live]') {
  const o = await open(code, opts);
  await o.page.waitForSelector(`main${ready}`, { timeout: 15000 });
  await o.page.waitForTimeout(250);
  await o.page.screenshot({ path: path.join(out, `made-${name}.png`), fullPage: (opts.width ?? 390) < 800 });
  await check(name, o, { width: opts.width ?? 390 });
  await o.ctx.close();
}

const TV = { width: 1920, height: 1080, hash: '#screen=t' };
const REMOTE = { hash: '#remote=t' };
try {
  // Phone, 390 px: every block between the two examples, the join, an empty and a locked list, hostile text, ended.
  await shot('phone-shelf', 'made-shelf');
  await shot('phone-shelf-empty', 'made-shelf-empty');
  await shot('phone-shelf-locked', 'made-shelf-locked');
  await shot('phone-shelf-join', 'made-shelf-join', {}, '[data-screen=join]');
  await shot('phone-opening', 'made-opening', { reduced: true });
  await shot('phone-opening-revealed', 'made-opening-revealed');
  await shot('phone-hostile', 'made-hostile');
  await shot('phone-ended', 'made-ended', {}, '[data-screen=ended]');
  // The states the examples don't reach: own list with Take back, a vote with hidden counts, a count-only list.
  await shot('phone-swap', 'made-swap');
  // Big screen, 1920x1080.
  await shot('screen-shelf', 'screen-made-shelf', TV);
  await shot('screen-opening', 'screen-made-opening', TV);
  await shot('screen-opening-revealed', 'screen-made-opening-revealed', TV);
  await shot('screen-hostile', 'screen-made-hostile', TV);
  await shot('screen-swap', 'screen-made-swap', TV);
  await shot('screen-used-link', 'screen-made-shelf', { ...TV, hash: '#screen=used' }, '[data-screen=signed-out]');
  // A smaller TV (1280x720): the same vw sizes must still fit with nothing cut off.
  const TV720 = { ...TV, width: 1280, height: 720 };
  await shot('screen-shelf-720', 'screen-made-shelf', TV720);
  await shot('screen-opening-revealed-720', 'screen-made-opening-revealed', TV720);
  // Big screen at 390 (a phone opening a screen link) must not clip either.
  await shot('screen-opening-390', 'screen-made-opening', { hash: '#screen=t' });
  // Host remote, 390 px.
  await shot('remote-opening', 'remote-made-opening', REMOTE);
  await shot('remote-shelf', 'remote-made-shelf', REMOTE);
  await shot('remote-used-link', 'remote-made-opening', { hash: '#remote=used' }, '[data-screen=signed-out]');

  // Walk 1: join, add a book, vote for it, take it back.
  {
    const o = await open('made-shelf-join');
    const p = o.page;
    await p.waitForSelector('main[data-screen=join]');
    await p.fill('#mname', 'Mo');
    await p.click('form button[type=submit]');
    await p.waitForSelector('main[data-screen=live] [data-block=board]');
    await p.fill('[data-block=form] input[name=title]', 'Middlemarch');
    await p.check('[data-block=form] input[name=genre] >> nth=0');
    await p.click('[data-block=form] button[type=submit]');
    await p.waitForSelector('.mdone:has-text("Middlemarch")');
    await p.click('.mvote:has-text("Middlemarch")');
    await p.waitForSelector('.mopt-row.on .mvote:has-text("Middlemarch")');
    const tally = await p.textContent('.mopt-row.on .mn');
    await p.screenshot({ path: path.join(out, 'made-walk-voted.png'), fullPage: true });
    const box = fake.box(o.url.split('/p/')[1].split('#')[0]);
    const sent = box.actions.map((a) => a.name);
    if (JSON.stringify(sent) !== JSON.stringify(['put', 'vote'])) problems.push(`walk: sent ${sent}`);
    if (tally !== '1') problems.push(`walk: tally ${tally}`);
    await check('walk-shelf', o, { width: 390 });
    await o.ctx.close();
  }
  // Walk 1b: one book per guest in the swap, so a second one is refused; the box's words show under the form.
  {
    const o = await open('made-swap');
    const p = o.page;
    await p.waitForSelector('[data-block=form]');
    await p.fill('[data-block=form] input[name=title]', 'Ulysses');
    await p.click('[data-block=form] button[type=submit]');
    await p.waitForSelector('[data-block=form] .err:has-text("You can add up to 1 here.")');
    await p.screenshot({ path: path.join(out, 'made-walk-refused.png'), fullPage: true });
    await check('walk-refused', o, { width: 390 });
    await o.ctx.close();
  }
  // Walk 1c: typing on through a push keeps the caret where it was (qa.md D1: "lemarchMidd").
  {
    const o = await open('made-shelf');
    const p = o.page;
    const code = o.url.split('/p/')[1].split('#')[0];
    await p.waitForSelector('[data-block=form] input[name=title]');
    await p.click('[data-block=form] input[name=title]');
    await p.keyboard.type('Midd');
    fake.push(code, (b) => b.rows.shelf.push({ id: 'px', owner: 'g2', values: { title: 'Pushed In', genre: 'novel' }, at: '' }));
    await p.waitForSelector('text=Pushed In');
    await p.keyboard.type('lemarch');
    const typed = await p.inputValue('[data-block=form] input[name=title]');
    if (typed !== 'Middlemarch') problems.push(`walk caret: typed through a push gave ${JSON.stringify(typed)}`);
    results['walk-caret'] = typed;
    await o.ctx.close();
  }
  // Walk 1d: 44 books on the big screen. Only whole rows that fit are drawn, then "and N books more" (qa.md D2).
  for (const [label, size] of [['1080', TV], ['720', TV720]]) {
    const o = await open('screen-made-shelf', size);
    const p = o.page;
    const code = o.url.split('/p/')[1].split('#')[0];
    await p.waitForSelector('[data-block=board]');
    fake.push(code, (b) => { for (let i = 0; i < 40; i++) b.rows.shelf.push({ id: `m${i}`, owner: 'g2', values: { title: `Book ${i + 1}`, genre: 'novel' }, at: '' }); });
    await p.waitForSelector('[data-block=board] .mmore');
    const facts = await p.evaluate(() => ['board', 'list'].map((k) => {
      const card = document.querySelector(`[data-block=${k}]`);
      return card && { k, rows: card.querySelectorAll('ul[data-total] > li').length, more: card.querySelector('.mmore')?.textContent ?? '' };
    }).filter(Boolean));
    for (const f of facts) {
      const m = /^and (\d+) \w+ more$/.exec(f.more);
      if (!m || f.rows + Number(m[1]) !== 44 || f.rows < 1) problems.push(`many ${label} ${f.k}: ${f.rows} rows, "${f.more}"`);
    }
    results[`many-${label}`] = facts;
    await p.screenshot({ path: path.join(out, `screen-many-${label}.png`) });
    await check(`screen-many-${label}`, o, { width: size.width });
    await o.ctx.close();
  }
  // Walk 2: a guess on the phone, then the host reveals from the remote and the phone shows the answers.
  {
    const phone = await open('made-opening');
    const pcode = phone.url.split('/p/')[1].split('#')[0];
    await phone.page.waitForSelector('[data-block=form]');
    await phone.page.fill('[data-block=form] input[name=text]', 'Nineteen Eighty-Four');
    await phone.page.click('[data-block=form] button[type=submit]');
    await phone.page.waitForSelector('.mdone:has-text("Nineteen Eighty-Four")');
    const sent = fake.box(pcode).actions.at(-1);
    if (sent?.name !== 'answer' || 'points' in (sent.body.values ?? {})) problems.push(`walk 2: sent ${JSON.stringify(sent)}`);
    // The host's reveal lands on the phone by the push.
    fake.push(pcode, (b) => (b.revealed = true));
    await phone.page.waitForSelector('.mreveal li');
    await phone.page.screenshot({ path: path.join(out, 'made-walk-revealed.png'), fullPage: true });
    await check('walk-opening', phone, { width: 390 });
    await phone.ctx.close();
    const remote = await open('remote-made-opening', REMOTE);
    await remote.page.waitForSelector('[data-block=reveal] button');
    await remote.page.click('[data-block=reveal] button');
    await remote.page.waitForSelector('[data-block=reveal] .mreveal li');
    await check('walk-remote', remote, { width: 390 });
    await remote.ctx.close();
  }
} catch (e) {
  problems.push(`crash: ${e.stack || e}`);
} finally {
  fs.writeFileSync(path.join(out, 'made-results.json'), JSON.stringify({ results, problems }, null, 2));
  await browser.close();
  await fake.close();
}
console.log(JSON.stringify(results, null, 1).slice(0, 4000));
if (problems.length) {
  console.error(`made e2e: ${problems.length} problem(s)\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log(`made e2e: ok, shots in ${out}`);
