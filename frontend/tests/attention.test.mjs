// "Needs you" nudges for the web app and PWA (attention.ts, NeedsYouPill.tsx, sw.js badge).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { loadBundled } from './bundle.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const a = await loadBundled(`export * from './src/attention';`);
const ui = await loadBundled(`
  export { NeedsYouPill } from './src/components/NeedsYouPill';
  export { createElement } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
`);

const item = (id, kind = 'question', link = { thread_id: null, channel_id: null, direct_kwant_id: 'k1' }, extra = {}) => ({
  kind, id, kwant_id: 'k1', kwant_name: 'Juno', title: `Title ${id}`, summary: '', status: 'open', timestamp: '2026-11-03T21:00:00Z', link, ...extra,
});
const view = (o = {}) => ({ openCtx: null, onInbox: false, hidden: false, enabled: true, ...o });

test('the first waiting list primes: what already waits never pops a pill', () => {
  const arr = a.createArrivals();
  assert.deepEqual(arr.fromAway([item('q1'), item('a1', 'approval')]), []);
  assert.equal(arr.primed, true);
});

test('only ids never seen before are arrivals; the same id again is not', () => {
  const arr = a.createArrivals();
  arr.fromAway([item('q1')]);
  const fresh = arr.fromAway([item('a2', 'approval'), item('q1')]);
  assert.deepEqual(fresh.map((n) => n.key), ['approval:a2']);
  assert.deepEqual(arr.fromAway([item('a2', 'approval'), item('q1')]), []);
  // Gone and back under the same id stays seen: an answered card never nudges twice.
  arr.fromAway([]);
  assert.deepEqual(arr.fromAway([item('q1')]), []);
});

test('a question and an approval with the same id are different things', () => {
  const arr = a.createArrivals();
  arr.fromAway([item('x')]);
  assert.deepEqual(arr.fromAway([item('x'), item('x', 'approval')]).map((n) => n.key), ['approval:x']);
});

test('finished, failed and handoff items never nudge', () => {
  const arr = a.createArrivals();
  arr.fromAway([]);
  assert.deepEqual(arr.fromAway([item('d1', 'delegation'), item('r1', 'routine_run')]), []);
});

test('an item belongs to its thread, channel or direct chat', () => {
  assert.equal(a.awayCtx(item('1', 'question', { thread_id: 't9', channel_id: 'c1', direct_kwant_id: null })), 't9');
  assert.equal(a.awayCtx(item('1', 'question', { thread_id: null, channel_id: 'c1', direct_kwant_id: null })), 'c1');
  assert.equal(a.awayCtx(item('1', 'question', { thread_id: null, channel_id: null, direct_kwant_id: null })), 'k1');
});

test('no pill for the chat you are looking at, on the Inbox, in a background tab, or with nudges off', () => {
  const n = { ctx: 'k1' };
  assert.equal(a.shouldPill(n, view()), true);
  assert.equal(a.shouldPill(n, view({ openCtx: 'k2' })), true);
  assert.equal(a.shouldPill(n, view({ openCtx: 'k1' })), false);
  assert.equal(a.shouldPill(n, view({ onInbox: true })), false);
  assert.equal(a.shouldPill(n, view({ hidden: true })), false);
  assert.equal(a.shouldPill(n, view({ enabled: false })), false);
});

test('a browser step is one arrival per bot and title a minute', () => {
  const arr = a.createArrivals();
  const ev = { kwant_id: 'k1', kwant_name: 'Juno', channel_id: null, title: 'Juno needs a hand', body: 'Solve the CAPTCHA' };
  const n = arr.fromHumanStep(ev, 1000);
  assert.equal(n.kind, 'human_step');
  assert.equal(n.ctx, 'k1');
  assert.equal(n.title, 'Solve the CAPTCHA');
  assert.equal(arr.fromHumanStep(ev, 30_000), null);
  assert.ok(arr.fromHumanStep(ev, 30_000 + a.STEP_DEDUPE_MS));
});

test('arrivals while the pill shows coalesce, newest first, without duplicates', () => {
  const arr = a.createArrivals();
  arr.fromAway([]);
  let s = a.pillAdd(a.EMPTY_PILL, arr.fromAway([item('q1')]));
  assert.equal(s.items.length, 1);
  const rev = s.rev;
  s = a.pillAdd(s, arr.fromAway([item('a2', 'approval'), item('q3'), item('q1')]));
  assert.deepEqual(s.items.map((n) => n.key), ['approval:a2', 'question:q3', 'question:q1']);
  assert.ok(s.rev > rev, 'the 5 s start again');
  assert.equal(a.pillAdd(s, [s.items[0]]), s, 'the same key twice changes nothing');
  assert.equal(a.pillText(s.items).headline, '3 things need you');
});

test('something answered elsewhere leaves the pill; browser steps stay until it closes', () => {
  const step = { key: 'human_step:k1:x', kind: 'human_step', kwantId: 'k1', kwantName: 'Juno', title: 'x', ctx: 'k1' };
  const s = a.pillAdd(a.EMPTY_PILL, [a.nudgeFromAway(item('q1')), a.nudgeFromAway(item('a2', 'approval')), step]);
  const pruned = a.pillPrune(s, new Set(['approval:a2']));
  assert.deepEqual(pruned.items.map((n) => n.key), ['approval:a2', 'human_step:k1:x']);
  assert.equal(a.pillPrune(pruned, new Set(['approval:a2'])), pruned);
});

test('the words: who and what for one, a count for several', () => {
  assert.equal(a.pillText([a.nudgeFromAway(item('a1', 'approval'))]).headline, 'Juno needs your OK');
  assert.equal(a.pillText([a.nudgeFromAway(item('q1'))]).headline, 'Juno asked you something');
  assert.equal(a.pillText([a.nudgeFromAway(item('q1'))]).detail, 'Title q1');
  assert.equal(a.pillText([{ kind: 'human_step', kwantName: 'Juno', title: 'x' }]).headline, 'Juno needs you to do a step');
  assert.equal(a.pillText([a.nudgeFromAway(item('q1')), a.nudgeFromAway(item('q2'))]).detail, 'All from Juno');
  assert.equal(a.fromNames([{ kwantName: 'Juno' }, { kwantName: 'Lin' }]), 'Juno and Lin');
  assert.equal(a.fromNames([{ kwantName: 'Juno' }, { kwantName: 'Lin' }, { kwantName: 'Vega' }]), 'Juno, Lin and Vega');
  assert.equal(a.fromNames(['A', 'B', 'C', 'D', 'E'].map((kwantName) => ({ kwantName }))), 'A, B and 3 more');
  assert.equal(a.pillText([{ kind: 'question', kwantName: '', title: '' }]).headline, 'A bot asked you something');
  assert.equal(a.pillText([]).headline, '');
});

const env = ({ stored = null, reduced = false, nav } = {}) => {
  const store = new Map(stored === null ? [] : [[a.NUDGES_KEY, stored]]);
  const events = [];
  return {
    store,
    events,
    env: {
      navigator: nav,
      matchMedia: (q) => ({ matches: reduced && q === '(prefers-reduced-motion: reduce)' }),
      localStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
      dispatchEvent: (e) => events.push(e.type) > 0,
    },
  };
};

test('"Show nudges" is on by default, remembers off, and tells the app', () => {
  assert.equal(a.nudgesEnabled(env().env), true);
  const e = env();
  a.setNudgesEnabled(false, e.env);
  assert.equal(e.store.get(a.NUDGES_KEY), 'off');
  assert.equal(a.nudgesEnabled(e.env), false);
  assert.deepEqual(e.events, [a.NUDGES_SETTING_EVENT]);
  assert.equal(a.nudgesEnabled({ localStorage: { getItem: () => { throw new Error('blocked'); } } }), true);
});

test('Reduce Motion: the pill fades instead of dropping', () => {
  assert.equal(a.pillMotion(env().env), 'drop');
  assert.equal(a.pillMotion(env({ reduced: true }).env), 'fade');
  assert.equal(a.pillMotion({}), 'drop');
});

test('app badge: feature-detected, the exact count, cleared at 0, never throws', async () => {
  assert.equal(a.canBadge({}), false);
  assert.equal(a.setAppBadgeCount(3, {}), 'unsupported');
  assert.equal(a.setAppBadgeCount(3, { navigator: {} }), 'unsupported');
  const calls = [];
  const nav = {
    setAppBadge(n) { if (this !== nav) throw new TypeError('Illegal invocation'); calls.push(['set', n]); return Promise.resolve(); },
    clearAppBadge() { calls.push(['clear']); return Promise.resolve(); },
  };
  assert.equal(a.canBadge({ navigator: nav }), true);
  assert.equal(a.setAppBadgeCount(2, { navigator: nav }), 'set');
  assert.equal(a.setAppBadgeCount(0, { navigator: nav }), 'cleared');
  assert.deepEqual(calls, [['set', 2], ['clear']]);
  // Only setAppBadge (older Safari): 0 clears it.
  const only = { setAppBadge: (n) => { calls.push(['only', n]); return Promise.resolve(); } };
  assert.equal(a.setAppBadgeCount(0, { navigator: only }), 'cleared');
  assert.deepEqual(calls.at(-1), ['only', 0]);
  // A rejected promise (permission, not installed) is swallowed.
  const rejecting = { setAppBadge: () => Promise.reject(new Error('NotAllowed')) };
  assert.equal(a.setAppBadgeCount(1, { navigator: rejecting }), 'set');
  const throwing = { setAppBadge: () => { throw new Error('nope'); } };
  assert.equal(a.setAppBadgeCount(1, { navigator: throwing }), 'unsupported');
  await new Promise((r) => setTimeout(r, 0));
});

test('kwant\'s own icon uses Mira\'s "!" SVG; any other icon is drawn', () => {
  assert.equal(a.markedIconFor('/icon.svg'), '/icon-needs-you.svg');
  assert.equal(a.markedIconFor('https://kwant.example/icon.svg'), '/icon-needs-you.svg');
  assert.equal(a.markedIconFor('/other.png'), null);
  const svg = readFileSync(`${root}/public/icon-needs-you.svg`, 'utf8');
  assert.doesNotMatch(svg, /<script|\son\w+=/i);
});

test('an SVG "!" icon keeps the SVG type', async () => {
  const link = { href: '/icon.svg', type: 'image/svg+xml' };
  const icon = a.createNudgeIcon({ iconLink: () => link, drawMarked: async () => '/icon-needs-you.svg' });
  await icon.set(true);
  assert.deepEqual(link, { href: '/icon-needs-you.svg', type: 'image/svg+xml' });
});

test('the "!" favicon goes on while something waits and gives the icon back after', async () => {
  const link = { href: '/icon.svg', type: 'image/svg+xml' };
  let draws = 0;
  const icon = a.createNudgeIcon({ iconLink: () => link, drawMarked: async () => (draws++, 'data:image/png;base64,MARK') });
  await icon.set(true);
  assert.equal(link.href, 'data:image/png;base64,MARK');
  assert.equal(link.type, 'image/png');
  await icon.set(true);
  await icon.set(false);
  assert.deepEqual(link, { href: '/icon.svg', type: 'image/svg+xml' });
  await icon.set(true);
  assert.equal(draws, 1, 'drawn once, reused');
  // Something else changed the icon meanwhile (the working dot): left alone.
  link.href = 'data:image/png;base64,DOT';
  await icon.set(false);
  assert.equal(link.href, 'data:image/png;base64,DOT');
});

test('the "!" favicon comes back off when the link reads its href back absolute, like a real <link>', async () => {
  // A browser's HTMLLinkElement.href always returns the resolved URL, whatever was set.
  const base = 'https://kwant.example/chat/abc';
  let raw = '/icon.svg';
  const link = { type: 'image/svg+xml', get href() { return new URL(raw, base).href; }, set href(v) { raw = v; } };
  const icon = a.createNudgeIcon({ iconLink: () => link, drawMarked: async () => '/icon-needs-you.svg' });
  await icon.set(true);
  assert.equal(link.href, 'https://kwant.example/icon-needs-you.svg');
  await icon.set(false);
  assert.equal(link.href, 'https://kwant.example/icon.svg', 'the last answer gives the normal icon back');
  assert.ok(a.sameUrl('/x.svg', '/x.svg'));
  assert.ok(!a.sameUrl('/x.svg', '/y.svg'));
});

test('the favicon draw resolving after nothing waits any more leaves the icon alone', async () => {
  const link = { href: '/icon.svg' };
  let release;
  const icon = a.createNudgeIcon({ iconLink: () => link, drawMarked: () => new Promise((r) => (release = r)) });
  const on = icon.set(true);
  await icon.set(false);
  release('data:x');
  await on;
  assert.equal(link.href, '/icon.svg');
});

// The component, rendered to markup.

const render = (items, matchMedia) => {
  const before = globalThis.matchMedia;
  globalThis.matchMedia = matchMedia;
  try {
    return ui.renderToStaticMarkup(ui.createElement(ui.NeedsYouPill, { items, rev: 1, spriteOf: () => 'violet', onOpen: () => {}, onDismiss: () => {} }));
  } finally {
    globalThis.matchMedia = before;
  }
};

test('the pill is a polite live region with a button that goes there, and no Allow or Deny', () => {
  const html = render([a.nudgeFromAway(item('a1', 'approval'))]);
  assert.match(html, /role="status"/);
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /Juno needs your OK/);
  assert.match(html, /Title a1/);
  assert.match(html, /Open the chat/);
  assert.match(html, /Escape dismisses/);
  assert.match(html, /data-mood="needs_you"/);
  assert.doesNotMatch(html, /Allow|Deny|Approve/i);
  assert.match(html, /data-motion="drop"/);
});

test('the pill under Reduce Motion is marked to fade', () => {
  const html = render([a.nudgeFromAway(item('q1'))], (q) => ({ matches: q === '(prefers-reduced-motion: reduce)' }));
  assert.match(html, /data-motion="fade"/);
});

test('the pill is always dark, whatever the OS colour scheme', () => {
  for (const scheme of ['light', 'dark']) {
    const html = render([a.nudgeFromAway(item('q1'))], (q) => ({ matches: q === `(prefers-color-scheme: ${scheme})` }));
    assert.doesNotMatch(html, /data-theme/);
  }
  const css = readFileSync(`${root}/src/styles/needsYou.css`, 'utf8');
  assert.doesNotMatch(css, /data-theme|prefers-color-scheme/);
  assert.doesNotMatch(css, /#ffffff/i);
  assert.match(css, /--kw-n-bg: #231d4b/);
  const src = readFileSync(`${root}/src/components/NeedsYouPill.tsx`, 'utf8');
  assert.doesNotMatch(src, /prefers-color-scheme/);
});

test('the hold: 5 s from the last arrival, never past 12 s from first showing, 2 s after a hover or focus leaves', () => {
  assert.equal(a.pillHoldMs(0, false), 5000);
  assert.equal(a.pillHoldMs(8000, false), 4000);
  assert.equal(a.pillHoldMs(11000, false), 1000);
  assert.equal(a.pillHoldMs(13000, false), 0);
  assert.equal(a.pillHoldMs(13000, true), 2000);
  assert.equal(a.pillHoldMs(1000, true), 2000);
});

test('several in the pill read as a count that opens the Inbox', () => {
  const html = render([a.nudgeFromAway(item('q1')), a.nudgeFromAway(item('a2', 'approval'))]);
  assert.match(html, /2 things need you/);
  assert.match(html, /Open the Inbox/);
  assert.match(html, /data-count="2"/);
});

test('nothing waiting: the live region stays, empty', () => {
  const html = render([]);
  assert.match(html, /role="status"/);
  assert.doesNotMatch(html, /kw-nudge-main/);
});

test('reduced-motion CSS turns the drop and the hop off', () => {
  const css = readFileSync(`${root}/src/styles/needsYou.css`, 'utf8');
  const block = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
  assert.match(block, /kw-nudge-fade/);
  assert.match(block, /\.kw-nudge__av, \.kw-nudge__av \.kw-mood, [^{]*\{ animation: none; \}/);
  // Tap targets: 52 px desktop, 56 px phone.
  assert.match(css, /height: 52px/);
  assert.match(css, /height: 56px/);
});

// sw.js: a needs-you push badges the app icon even while kwant is closed.

function worker({ badge = true, front = false } = {}) {
  const code = readFileSync(`${root}/sw.js`, 'utf8').replace('__KWNT_SW_VERSION__', 'v').replace('__KWNT_SW_SHELL__', '[]');
  const handlers = {};
  const log = { badged: 0, shown: 0 };
  const self = {
    location: { origin: 'https://kwant.example' },
    addEventListener: (type, fn) => { handlers[type] = fn; },
    navigator: badge ? { setAppBadge: () => { log.badged++; return Promise.resolve(); } } : {},
    registration: { showNotification: async () => { log.shown++; } },
    clients: {
      matchAll: async () => (front ? [{ url: 'https://kwant.example/', focused: true, visibilityState: 'visible' }] : []),
      openWindow: async () => undefined,
      claim: async () => undefined,
    },
  };
  vm.runInNewContext(code, { self, URL, caches: {}, fetch: async () => ({ ok: true }) });
  return {
    log,
    push: async (kind) => {
      let waited;
      handlers.push({ data: { json: () => ({ title: 't', kind }) }, waitUntil: (p) => { waited = p; } });
      await waited;
    },
  };
}

test('sw.js: approvals, questions and browser steps badge the icon; other pushes do not', async () => {
  for (const kind of ['approval_needed', 'question_asked', 'human_step']) {
    const w = worker();
    await w.push(kind);
    assert.equal(w.log.badged, 1, kind);
    assert.equal(w.log.shown, 1);
  }
  const other = worker();
  await other.push('delegation_finished');
  assert.equal(other.log.badged, 0);
});

test('sw.js: no badge API, or kwant in front, still works without one', async () => {
  const none = worker({ badge: false });
  await none.push('approval_needed');
  assert.equal(none.log.shown, 1);
  const front = worker({ front: true });
  await front.push('approval_needed');
  assert.equal(front.log.badged, 0, 'the open page sets the exact count itself');
});
