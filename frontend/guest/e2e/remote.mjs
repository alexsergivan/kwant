// pa4b host remote in a real Chrome under pa1's pocket CSP, from the fake guest port (guest/fake/remote.mjs).
// Fails on: a CSP violation, a page error, a request that is not same-origin, sideways scrolling at 390 px, a visible
// tap target under 44 px, the #remote= token still in the address after load, or a holo sticker on the remote.
// Writes a 390 px (DPR 2) shot of every remote tab, sheet and screen.
// Usage: PW_CORE=<playwright-core/index.mjs> CHROMIUM=<chrome> node guest/e2e/remote.mjs <out dir>
// Needs the built page in frontend/dist/pocket (npm run build:guest).
import fs from 'node:fs';
import path from 'node:path';
import { startFake } from '../fake/server.mjs';
import { CSP } from '../fake/views.mjs';

const pw = await import(process.env.PW_CORE || 'playwright-core');
const out = path.resolve(process.argv[2] || 'remote-shots');
fs.mkdirSync(out, { recursive: true });

const fake = await startFake({ port: 0, actionDelayMs: 300 });
const origin = `http://127.0.0.1:${fake.port}`;
const browser = await pw.chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ['--no-sandbox'] });
const problems = [];
const results = { shots: [] };

let seq = 0;
async function open(scene, token = `tok-${scene}-${++seq}`) {
  const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, locale: 'en-US', reducedMotion: 'reduce' });
  await ctx.addInitScript(() => {
    window.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(`${e.violatedDirective} ${e.blockedURI || ''}`.trim()));
  });
  const page = await ctx.newPage();
  const log = [];
  page.on('console', (m) => /Content.Security.Policy|Refused to/i.test(m.text()) && log.push(`console: ${m.text()}`));
  page.on('pageerror', (e) => log.push(`pageerror: ${e.message}`));
  page.on('request', (r) => {
    const u = r.url();
    if (!u.startsWith('http://127.0.0.1:') && !u.startsWith('ws://127.0.0.1:') && !u.startsWith('data:')) log.push(`third-party request: ${u}`);
    if (/[?&]token=|tok-/.test(u.split('#')[0])) log.push(`token in a request URL: ${u}`);
  });
  const res = await page.goto(`${origin}/p/remote-${scene}#remote=${token}`);
  if (res.headers()['content-security-policy'] !== CSP) problems.push(`${scene}: wrong CSP header`);
  return { ctx, page, log };
}

const settled = (page, screen) =>
  page.waitForSelector(`[data-screen="${screen}"]`, { state: 'attached', timeout: 15000 }).catch(async (e) => {
    const now = await page.evaluate(() => [location.pathname, document.querySelector('[data-screen]')?.getAttribute('data-screen'), document.body.innerText.slice(0, 160)]);
    throw new Error(`waited for ${screen}, page shows ${JSON.stringify(now)}`, { cause: e });
  });

async function check(name, page, log) {
  for (const x of await page.evaluate(() => window.__violations)) problems.push(`${name}: CSP violation: ${x}`);
  for (const x of log) problems.push(`${name}: ${x}`);
  const f = await page.evaluate(() => {
    const small = [];
    for (const el of document.querySelectorAll('button, a, [role=tab]')) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || el.closest('[inert]')) continue;
      if (r.height < 44 || r.width < 44) small.push(`${el.tagName.toLowerCase()}.${el.className} "${(el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 24)}" ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    return { scrollWidth: document.documentElement.scrollWidth, small, hash: location.hash, holo: document.querySelectorAll('.kw-sticker-holo, [class*=holo]').length };
  });
  if (f.scrollWidth > 390) problems.push(`${name}: scrolls sideways (${f.scrollWidth} px)`);
  if (f.hash.includes('remote=')) problems.push(`${name}: token still in the address (${f.hash})`);
  if (f.holo) problems.push(`${name}: holo on the remote`);
  for (const s of f.small) problems.push(`${name}: tap target under 44 px: ${s}`);
}

async function shot(name, page, log) {
  await page.waitForTimeout(300);
  await check(name, page, log);
  // A sheet is fixed to the bottom of the phone screen, so it is shot at screen size, as a phone shows it.
  const sheet = await page.$('[role=dialog]');
  await page.screenshot({ path: path.join(out, `remote-${name}-390.png`), fullPage: !sheet });
  results.shots.push(name);
}

const live = async (scene, name = scene) => {
  const o = await open(scene);
  await settled(o.page, 'live');
  await shot(name, o.page, o.log);
  return o;
};

for (const s of ['lobby', 'empty', 'question', 'locked', 'miss', 'paused', 'reveal', 'leaderboard', 'last', 'solo']) {
  const { ctx } = await live(s);
  await ctx.close();
}

// Tabs, Undo line, Remove sheet, End confirm, from one live question.
{
  const { ctx, page, log } = await live('question', 'question-game');
  await page.click('[data-k="tab-teams"]');
  await shot('teams', page, log);
  const plus = page.locator('[data-k^="plus-"]').first();
  await plus.click();
  await page.waitForSelector('[data-k="undo"]');
  await shot('teams-undo', page, log);
  await page.click('[data-k="tab-players"]');
  await shot('players', page, log);
  await page.click('[data-k="more-p9"]');
  await page.waitForSelector('[role=dialog]');
  const focus = await page.evaluate(() => document.activeElement?.getAttribute('data-k'));
  if (focus !== 'sheet-remove') problems.push(`remove sheet: focus on ${focus}, not the first button`);
  await shot('remove-sheet', page, log);
  await page.keyboard.press('Escape');
  const back = await page.evaluate(() => document.activeElement?.getAttribute('data-k'));
  if (back !== 'more-p9') problems.push(`remove sheet: Escape left focus on ${back}`);
  await page.click('[data-k="tab-game"]');
  await page.click('[data-k="end"]');
  await page.waitForSelector('[role=dialog]');
  await shot('end-confirm', page, log);
  await page.click('[data-k="sheet-end"]');
  await settled(page, 'live');
  await page.waitForTimeout(600);
  await shot('ended', page, log);
  // A reload with no fragment reads host/state with the cookie and stays signed in.
  await page.reload();
  await settled(page, 'live');
  await check('reload', page, log);
  // Signed out: the cookie is gone, no token in the address.
  await ctx.clearCookies();
  await page.reload();
  await settled(page, 'signed-out');
  await shot('signed-out', page, log);
  await ctx.close();
}

for (const [scene, screen, token] of [['expired', 'expired'], ['question', 'screen-only', 'screen']]) {
  const { ctx, page, log } = await open(scene, token);
  await settled(page, screen);
  await shot(screen === 'expired' ? 'link-expired' : 'screen-only', page, log);
  await ctx.close();
}
{
  const { ctx, page, log } = await open('away');
  await page.waitForTimeout(1500);
  await shot('away', page, log);
  await ctx.close();
}

// Widths (Rigel round 1, D1): the longest legal names (player MAX_NAME 20, team MAX_TEAM_NAME 20) at 390 and
// 320 px. Every button, status and score must sit inside the window, not only the page's scrollWidth: the list
// clips, so overflow there never shows up as page scroll.
{
  const b = fake.box('remote-wide');
  b.players = Array.from({ length: 8 }, (_, i) => ({ id: `p${i + 1}`, name: i < 4 ? 'W'.repeat(20) : 'Mia', team: i < 4 ? `M${i}`.padEnd(20, 'M') : 'Fact Hunters', connected: true, removed: false, score: 123456, avatar: null, joined_at: null }));
  b.answers = new Set(['p1']);
  b.pending = [{ q: 3, player: 'p1', question: 'What?', answer: 'W'.repeat(40), accept: [], guess: 'M'.repeat(40) }];
  results.widths = [];
  for (const w of [390, 320]) {
    const { ctx, page, log } = await open('wide');
    await page.setViewportSize({ width: w, height: 844 });
    await settled(page, 'live');
    for (const tab of ['game', 'teams', 'players']) {
      await page.click(`[data-k="tab-${tab}"]`);
      await page.waitForTimeout(150);
      const m = await page.evaluate((w) => ({
        sw: document.documentElement.scrollWidth,
        off: [...document.querySelectorAll('.rm button, .rm .st, .rm .sc')]
          .filter((e) => { const r = e.getBoundingClientRect(); return r.width && (r.right > w + 1 || r.left < -1); })
          .map((e) => e.getAttribute('data-k') || e.className).slice(0, 6),
        // Text in a row must end before the row's next control (the status or the button), not run under it.
        overlap: [...document.querySelectorAll('.rrowi .who')].filter((who) => {
          const next = who.nextElementSibling; if (!next) return false;
          const end = Math.max(...[...who.children].map((c) => c.getBoundingClientRect().right));
          return end > next.getBoundingClientRect().left + 1;
        }).length,
      }), w);
      const ok = m.sw <= w && !m.off.length && !m.overlap;
      results.widths.push({ width: w, tab, ok, ...m });
      if (!ok) problems.push(`widths ${w} ${tab}: ${m.sw > w ? `scrolls sideways (${m.sw} px)` : ''} off-screen: ${m.off.join(', ')}${m.overlap ? ` · ${m.overlap} rows with text under the next control` : ''}`);
      if (w === 320 || tab !== 'game') await page.screenshot({ path: path.join(out, `remote-wide-${tab}-${w}.png`), fullPage: true });
    }
    for (const x of log) problems.push(`widths ${w}: ${x}`);
    await ctx.close();
  }
}

results.problems = problems;
fs.writeFileSync(path.join(out, 'remote-e2e.json'), JSON.stringify(results, null, 2));
await browser.close();
await fake.close();
console.log(JSON.stringify(results, null, 2));
process.exit(problems.length ? 1 : 0);
